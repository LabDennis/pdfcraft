//! Convert colours (Acrobat's Print production ▸ Convert Colors): every colour
//! operator in the page content and the artwork it draws is rewritten in a target space -
//! grayscale, RGB or CMYK - by the standard device formulas (no ICC profile is applied), black
//! preserved on request; pictures are re-encoded through a helper the caller supplies.

use pdfcraft_content::{Op, parse, serialize_ops};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};
use std::collections::{HashMap, HashSet};

use crate::colour::{Space, rgb, space_of};
use crate::{EditError, check, page_list};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourTarget {
    Gray,
    Rgb,
    Cmyk,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertColours {
    pub target: ColourTarget,
    /// Pure black stays 100% K (or gray 0) rather than a mix.
    pub preserve_black: bool,
    /// Re-encode pictures too (through the caller's helper).
    pub images: bool,
    /// The appearance streams of the pages' comments and form fields too.
    pub annotations: bool,
}

impl Default for ConvertColours {
    fn default() -> Self {
        Self { target: ColourTarget::Gray, preserve_black: true, images: true, annotations: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Converted {
    /// Colour operators rewritten.
    pub colours: usize,
    /// Pictures re-encoded.
    pub images: usize,
    /// Pages on which something changed.
    pub pages: usize,
}

/// Re-encode one image stream in the target space, if it can be: the caller's (the optimizer
/// knows how to decode and encode pictures; the edit crate does not).
pub type ImageRecolour<'a> = dyn FnMut(&Document, &Stream, ColourTarget) -> Option<Stream> + 'a;

fn luma(c: [f64; 3]) -> f64 {
    (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]).clamp(0.0, 1.0)
}

/// sRGB → the target's components.
pub fn components(target: ColourTarget, c: [f64; 3], preserve_black: bool) -> Vec<f64> {
    let black = c.iter().all(|v| *v < 0.02);
    match target {
        ColourTarget::Gray => vec![if black && preserve_black { 0.0 } else { luma(c) }],
        ColourTarget::Rgb => c.to_vec(),
        ColourTarget::Cmyk => {
            if black && preserve_black {
                return vec![0.0, 0.0, 0.0, 1.0];
            }
            let k = 1.0 - c.iter().cloned().fold(0.0, f64::max);
            if k >= 0.999 {
                return vec![0.0, 0.0, 0.0, 1.0];
            }
            let f = |v: f64| ((1.0 - v - k) / (1.0 - k)).clamp(0.0, 1.0);
            vec![f(c[0]), f(c[1]), f(c[2]), k]
        }
    }
}

fn op_for(target: ColourTarget, stroke: bool) -> &'static str {
    match (target, stroke) {
        (ColourTarget::Gray, false) => "g",
        (ColourTarget::Gray, true) => "G",
        (ColourTarget::Rgb, false) => "rg",
        (ColourTarget::Rgb, true) => "RG",
        (ColourTarget::Cmyk, false) => "k",
        (ColourTarget::Cmyk, true) => "K",
    }
}

fn r3(v: f64) -> Object {
    Object::Real((v * 1000.0).round() / 1000.0)
}

/// Rewrite one content stream's colour operators. `cs_res` is its `/ColorSpace` resource
/// dictionary. Returns the new bytes when anything changed, and how many operators.
fn rewrite(data: &[u8], doc: &Document, cs_res: &Dict, settings: &ConvertColours) -> (Option<Vec<u8>>, usize) {
    let mut ops: Vec<Op> = parse(data).ops;
    let mut count = 0usize;
    let mut out: Vec<Op> = Vec::with_capacity(ops.len());
    // The current fill and stroke spaces (None = a device space set by g/rg/k, which the
    // converted operator sets again), with the q/Q stack.
    let mut fill: Option<Space> = None;
    let mut stroke: Option<Space> = None;
    let mut stack: Vec<(Option<Space>, Option<Space>)> = Vec::new();
    let numbers = |op: &Op| -> Vec<f64> { op.operands.iter().filter_map(Object::as_f64).collect() };
    let convert = |c: [f64; 3], is_stroke: bool| -> Op {
        Op::new(op_for(settings.target, is_stroke), components(settings.target, c, settings.preserve_black).into_iter().map(r3).collect())
    };
    for op in ops.drain(..) {
        match op.op.as_slice() {
            b"q" => {
                stack.push((fill.clone(), stroke.clone()));
                if stack.len() > 512 {
                    stack.remove(0);
                }
                out.push(op);
            }
            b"Q" => {
                if let Some((f, s)) = stack.pop() {
                    fill = f;
                    stroke = s;
                }
                out.push(op);
            }
            b"cs" | b"CS" => {
                let is_stroke = op.op.as_slice() == b"CS";
                let space = op.operands.first().map(|o| space_of(doc, o, cs_res, 0)).unwrap_or(Space::Gray);
                let pattern = space == Space::Pattern;
                if is_stroke {
                    stroke = Some(space);
                } else {
                    fill = Some(space);
                }
                if pattern {
                    // Patterns stay what they are (their own content is not converted here).
                    out.push(op);
                } else {
                    // The space's initial colour, now in the target: black (a tint at full).
                    let space = if is_stroke { stroke.clone() } else { fill.clone() }.unwrap_or(Space::Gray);
                    let initial = match space {
                        Space::Tint { .. } => rgb(&space, &[1.0; 8]),
                        _ => rgb(&space, &[]),
                    };
                    out.push(convert(initial, is_stroke));
                    count += 1;
                }
            }
            b"g" | b"rg" | b"k" | b"G" | b"RG" | b"K" => {
                let is_stroke = op.op[0].is_ascii_uppercase();
                let space = match op.op.to_ascii_lowercase().as_slice() {
                    b"g" => Space::Gray,
                    b"rg" => Space::Rgb,
                    _ => Space::Cmyk,
                };
                let already = matches!(
                    (settings.target, &space),
                    (ColourTarget::Gray, Space::Gray) | (ColourTarget::Rgb, Space::Rgb) | (ColourTarget::Cmyk, Space::Cmyk)
                );
                if is_stroke {
                    stroke = None;
                } else {
                    fill = None;
                }
                if already {
                    out.push(op);
                } else {
                    let c = rgb(&space, &numbers(&op));
                    out.push(convert(c, is_stroke));
                    count += 1;
                }
            }
            b"sc" | b"scn" | b"SC" | b"SCN" => {
                let is_stroke = op.op[0].is_ascii_uppercase();
                let space = if is_stroke { stroke.clone() } else { fill.clone() };
                match space {
                    Some(Space::Pattern) | None if op.operands.iter().any(|o| matches!(o, Object::Name(_))) => out.push(op),
                    None => {
                        // sc after g/rg/k: the device space those set; read the numbers by count.
                        let n = numbers(&op);
                        let space = match n.len() {
                            1 => Space::Gray,
                            4 => Space::Cmyk,
                            _ => Space::Rgb,
                        };
                        out.push(convert(rgb(&space, &n), is_stroke));
                        count += 1;
                    }
                    Some(Space::Pattern) => out.push(op),
                    Some(space) => {
                        out.push(convert(rgb(&space, &numbers(&op)), is_stroke));
                        count += 1;
                    }
                }
            }
            _ => out.push(op),
        }
    }
    ((count > 0).then(|| serialize_ops(&out)), count)
}

fn stream_of(doc: &Document, r: ObjRef) -> Option<Stream> {
    match &*doc.get(r) {
        Object::Stream(s) => Some(s.clone()),
        _ => None,
    }
}

fn cs_resources(doc: &Document, resources: Option<&Dict>) -> Dict {
    resources.and_then(|r| r.get(b"ColorSpace")).map(|c| doc.resolve(c)).and_then(|c| c.as_dict().cloned()).unwrap_or_default()
}

// Keep the transaction bounded even when resources form a deep or cyclic graph.
const MAX_COLOUR_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLOUR_OBJECTS: usize = 10_000;

struct Conversion<'a> {
    settings: &'a ConvertColours,
    recolour: &'a mut ImageRecolour<'a>,
    copied: HashMap<ObjRef, ObjRef>,
    visiting: HashSet<ObjRef>,
    bytes_left: usize,
    result: Converted,
}

impl Conversion<'_> {
    fn decode(&mut self, stream: &Stream) -> Result<Vec<u8>, EditError> {
        let bytes = stream.decoded_within(self.bytes_left)?;
        self.bytes_left = self.bytes_left.checked_sub(bytes.len()).ok_or_else(|| EditError::Invalid("colour conversion is too large".into()))?;
        Ok(bytes)
    }

    fn resources(&mut self, doc: &mut Document, resources: &Dict, depth: usize) -> Result<Dict, EditError> {
        let mut result = resources.clone();
        if let Some(mut objects) = resources.get(b"XObject").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()) {
            let entries: Vec<_> = objects.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
            if entries.len() > MAX_COLOUR_OBJECTS {
                return Err(EditError::Invalid("too many colour resources".into()));
            }
            let mut changed = false;
            for (name, value) in entries {
                if let Some(old) = value.as_ref() {
                    let new = self.object(doc, old, depth)?;
                    if new != old {
                        objects.set(name, Object::Ref(new));
                        changed = true;
                    }
                }
            }
            if changed {
                result.set(b"XObject".to_vec(), Object::Dict(objects));
            }
        }
        Ok(result)
    }

    // Copy only modified XObjects; never overwrite a source shared with another page.
    fn object(&mut self, doc: &mut Document, old: ObjRef, depth: usize) -> Result<ObjRef, EditError> {
        if let Some(new) = self.copied.get(&old) {
            return Ok(*new);
        }
        if depth >= 64 || self.copied.len() + self.visiting.len() >= MAX_COLOUR_OBJECTS || !self.visiting.insert(old) {
            return Err(EditError::Invalid("colour resources are cyclic or too complex".into()));
        }
        let stream = stream_of(doc, old).ok_or_else(|| EditError::Invalid("a colour resource is not a stream".into()))?;
        let new = match stream.dict.name(b"Subtype") {
            Some(b"Form") => {
                let resources = stream.dict.get(b"Resources").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()).unwrap_or_default();
                let data = self.decode(&stream)?;
                let (content, count) = rewrite(&data, doc, &cs_resources(doc, Some(&resources)), self.settings);
                let converted = self.resources(doc, &resources, depth + 1)?;
                if content.is_some() || converted != resources {
                    let mut dict = stream.dict.clone();
                    if converted != resources {
                        dict.set(b"Resources".to_vec(), Object::Dict(converted));
                    }
                    let changed = if let Some(bytes) = content {
                        for key in [&b"Length"[..], b"Filter", b"DecodeParms"] {
                            dict.remove(key);
                        }
                        Stream::flate(dict, &bytes)
                    } else {
                        Stream { dict, raw: stream.raw.clone() }
                    };
                    self.result.colours += count;
                    doc.add(Object::Stream(changed))
                } else {
                    old
                }
            }
            Some(b"Image") if self.settings.images => {
                if let Some(changed) = (self.recolour)(doc, &stream, self.settings.target) {
                    self.result.images += 1;
                    doc.add(Object::Stream(changed))
                } else {
                    old
                }
            }
            _ => old,
        };
        self.visiting.remove(&old);
        self.copied.insert(old, new);
        Ok(new)
    }

    fn appearances(&mut self, doc: &mut Document, appearance: &Dict) -> Result<Dict, EditError> {
        let mut result = appearance.clone();
        for (name, value) in appearance.iter() {
            match &*doc.resolve(value) {
                Object::Stream(_) => {
                    if let Some(old) = value.as_ref() {
                        let new = self.object(doc, old, 0)?;
                        if new != old {
                            result.set(name.clone(), Object::Ref(new));
                        }
                    }
                }
                Object::Dict(states) => {
                    let mut changed = states.clone();
                    for (state, value) in states.iter() {
                        if let Some(old) = value.as_ref() {
                            changed.set(state.clone(), Object::Ref(self.object(doc, old, 0)?));
                        }
                    }
                    if &changed != states {
                        result.set(name.clone(), Object::Dict(changed));
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }
}

fn annotations(doc: &Document, page: &pdfcraft_model::Page) -> Vec<ObjRef> {
    page.dict
        .get(b"Annots")
        .map(|o| doc.resolve(o))
        .and_then(|o| o.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(Object::as_ref)
        .collect()
}

/// Convert selected pages transactionally. Shared artwork stays unchanged for all other users.
pub fn convert_colours(
    doc: &mut Document,
    pages: &[usize],
    settings: &ConvertColours,
    recolour: &mut ImageRecolour<'_>,
) -> Result<Converted, EditError> {
    let all = page_list(doc);
    check(pages, all.len())?;
    let mut staged = doc.clone();
    let mut conversion = Conversion {
        settings,
        recolour,
        copied: HashMap::new(),
        visiting: HashSet::new(),
        bytes_left: MAX_COLOUR_BYTES,
        result: Converted::default(),
    };
    let mut seen_pages = HashSet::new();
    let mut annotation_owners = HashMap::<ObjRef, usize>::new();
    if settings.annotations {
        for page in &all {
            for annotation in annotations(doc, page) {
                *annotation_owners.entry(annotation).or_default() += 1;
            }
        }
    }
    for &index in pages {
        if !seen_pages.insert(index) {
            continue;
        }
        let page = all.get(index).ok_or(EditError::NoSuchPage(index))?;
        let resources = page.dict.get(b"Resources").map(|r| staged.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let entries = match page.dict.get(b"Contents").map(|c| staged.resolve(c)) {
            Some(content) => match &*content {
                Object::Array(entries) => entries.clone(),
                Object::Null => Vec::new(),
                _ => vec![(*content).clone()],
            },
            None => Vec::new(),
        };
        let mut joined = Vec::new();
        for entry in &entries {
            let content = staged.resolve(entry);
            let Object::Stream(stream) = &*content else {
                return Err(EditError::Invalid("page content is not a stream".into()));
            };
            let decoded = conversion.decode(stream)?;
            if joined.len().saturating_add(decoded.len()).saturating_add(1) > MAX_COLOUR_BYTES {
                return Err(EditError::Invalid("page colour content is too large".into()));
            }
            joined.extend_from_slice(&decoded);
            joined.push(b'\n');
        }
        let (content, count) = rewrite(&joined, &staged, &cs_resources(&staged, Some(&resources)), settings);
        let mut changed = content.is_some();
        if let Some(bytes) = content {
            let stream = staged.add(Object::Stream(Stream::flate(Dict::new(), &bytes)));
            staged.update_dict(page.obj, |d| d.set(b"Contents".to_vec(), Object::Ref(stream)))?;
            conversion.result.colours += count;
        }
        let converted = conversion.resources(&mut staged, &resources, 0)?;
        if converted != resources {
            staged.update_dict(page.obj, |d| d.set(b"Resources".to_vec(), Object::Dict(converted)))?;
            changed = true;
        }
        if settings.annotations {
            for annotation in annotations(doc, page) {
                if annotation_owners.get(&annotation).copied().unwrap_or(0) > 1 {
                    return Err(EditError::Invalid("an annotation is shared by multiple pages".into()));
                }
                let appearance = staged
                    .get(annotation)
                    .as_dict()
                    .and_then(|d| d.get(b"AP").cloned())
                    .map(|o| staged.resolve(&o))
                    .and_then(|o| o.as_dict().cloned());
                if let Some(appearance) = appearance {
                    let converted = conversion.appearances(&mut staged, &appearance)?;
                    if converted != appearance {
                        staged.update_dict(annotation, |d| d.set(b"AP".to_vec(), Object::Dict(converted)))?;
                        changed = true;
                    }
                }
            }
        }
        conversion.result.pages += usize::from(changed);
    }
    if conversion.result.pages > 0 {
        *doc = staged;
    }
    Ok(conversion.result)
}
