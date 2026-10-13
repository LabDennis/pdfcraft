//! Compare files in the real shell (egui_kittest): the dialog, the Compare panel, marks and
//! the report.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

#[test]
fn compare_two_versions() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare Files");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Compare));
    assert_eq!(h.state().toast.clone().unwrap().0, "2 differences found");
    h.get_by_label("2 Replaced");
    h.get_by_label("\"five\" → \"three\"").click();
    h.run_steps(2);
    assert_eq!(h.state().compare.as_ref().unwrap().selected, Some(0));
    let id = h.state().active_ids().unwrap().1;
    let i = h.state().active_ids().unwrap().0;
    assert_eq!(h.state().views[i].compare_marks.len(), 2);
    // Side by side: the old file's half shades its side of the changes and follows the new
    // file's zoom; "New file only" ends it.
    assert!(h.state().compare.as_ref().unwrap().side_by_side, "side by side on a wide window");
    let old_i = h.state().views.iter().position(|v| v.id == h.state().compare.as_ref().unwrap().old).unwrap();
    assert_eq!(h.state().views[old_i].compare_marks.len(), 2, "replaced words shaded on the old file too");
    assert!(h.state().views[old_i].flash.is_some(), "the old half jumped to the change");
    h.state_mut().views[i].zoom = 1.5;
    h.state_mut().views[i].fit = pdfcraft_ui_egui::canvas::Fit::None;
    h.run_steps(3);
    assert_eq!((h.state().views[old_i].zoom, h.state().views[old_i].fit), (1.5, pdfcraft_ui_egui::canvas::Fit::None), "the old half follows");
    h.get_by_label("New file only").click();
    h.run_steps(2);
    assert!(!h.state().compare.as_ref().unwrap().side_by_side);
    h.get_by_label("Side by side").click();
    h.run_steps(2);
    assert!(h.state().compare.as_ref().unwrap().side_by_side);
    h.get_by_label("New file only").click();
    h.run_steps(2);
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 2);
    h.get_by_label("Report…").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(h.state().active_ids().unwrap().1).unwrap().name, "Compare Report.pdf");
}

/// Writes a screenshot of the Compare panel when PDFCRAFT_SHOT is set (for review).
#[test]
fn compare_panel_screenshot() {
    let Ok(out) = std::env::var("PDFCRAFT_SHOT") else { return };
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().run_compare();
    for _ in 0..20 {
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.render().unwrap().save(out).unwrap();
}

#[test]
fn pdfa_dialog_verifies_and_converts() {
    let doc = Session::new().create_from_text("t", "Keep forever").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("keep.pdf", None, doc.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("standards.pdfa"));
    h.run_steps(2);
    h.get_by_label("Declared conformance: none");
    h.get_by_label("Verify").click();
    h.run_steps(3);
    h.get_by_label("The document has no XMP metadata");
    h.get_by_label("Save as PDF/A").click();
    h.run_steps(3);
    h.get_by_label("Declared conformance: PDF/A-2b");
    let issues = h.state().pdfa.issues.clone().unwrap();
    assert!(issues.iter().all(|i| !i.fixable), "{issues:?}");
    let id = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), Some("Save as PDF/A-2b"));
}

/// A one-page PDF whose XMP declares the given `pdfaid`/`pdfuaid` attributes.
fn declaring(ids: &str) -> Vec<u8> {
    let xmp = format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
         xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" {ids}/></rdf:RDF>"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Metadata 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << >> >>".to_string(),
        format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len()),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn pdfa_dialog_shows_a_declared_pdfua() {
    // The dialog read only the PDF/A identification, so a PDF/UA-1 document showed "none" (#1309).
    for (ids, shown) in [
        ("pdfuaid:part=\"1\"", "Declared conformance: PDF/UA-1"),
        ("pdfaid:part=\"2\" pdfaid:conformance=\"B\" pdfuaid:part=\"1\"", "Declared conformance: PDF/A-2b, PDF/UA-1"),
    ] {
        let doc = declaring(ids);
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.open_bytes("ua.pdf", None, doc.clone()).unwrap();
            app
        });
        h.run_steps(4);
        assert!(h.state_mut().execute("standards.pdfa"));
        h.run_steps(2);
        h.get_by_label(shown);
    }
}

#[test]
fn export_to_word_html_and_rtf() {
    let doc = Session::new().create_from_text("t", "Exported words").unwrap().to_vec();
    let dir = std::env::temp_dir().join(format!("pdfcraft-office-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("e.pdf", None, doc).unwrap();
    for ext in ["docx", "pptx", "html", "rtf"] {
        let out = dir.join(format!("e.{ext}"));
        app.save_override = Some(out.to_string_lossy().into_owned());
        assert!(app.execute(&format!("export.{ext}")));
        assert!(std::fs::metadata(&out).unwrap().len() > 40, "{ext}");
    }
    assert!(std::fs::read_to_string(dir.join("e.html")).unwrap().contains("Exported words"));
}

#[test]
fn compare_link_properties_edit_the_originating_document() {
    use pdfcraft_engine::{Edit, LinkAction, LinkStyle};
    for (source, reorder) in [(0, false), (0, true), (1, false)] {
        let bytes = Session::new().create_from_text("t", "A comparison fixture.").unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            for (name, url) in [("old.pdf", "https://example.org/old"), ("new.pdf", "https://example.org/new")] {
                app.open_bytes(name, None, bytes.as_ref().clone()).unwrap();
                assert!(app.apply_edit(Edit::AddLink {
                    page: 0,
                    rect: [20.0, 20.0, 100.0, 50.0],
                    action: LinkAction::Uri(url.into()),
                    style: LinkStyle::default(),
                }));
            }
            app
        });
        h.run_steps(4);
        let ids = [h.state().views[0].id, h.state().views[1].id];
        let urls = ["https://example.org/old", "https://example.org/new"];
        h.state_mut().compare_old = Some(ids[0]);
        h.state_mut().run_compare();
        h.run_steps(3);
        assert!(h.state().compare.as_ref().unwrap().side_by_side);
        assert_eq!(h.state().active, Some(1));
        // Both canvases expose distinct, usable toolbars to pointer and accessibility input.
        assert_eq!(h.query_all_by_label("Hand (H)").count(), 2);
        for toolbar in 0..2 {
            h.get_all_by_label("Hand (H)").nth(toolbar).unwrap().click();
            h.run_steps(2);
            assert_eq!(h.state().quick_tool, pdfcraft_ui_egui::QuickTool::Hand);
            h.get_all_by_label("Select (V)").nth(toolbar).unwrap().click();
            h.run_steps(2);
            assert_eq!(h.state().quick_tool, pdfcraft_ui_egui::QuickTool::Select);
        }
        // The request produced by double-clicking a link in either comparison canvas.
        h.state_mut().views[source].links.open_existing = Some((0, 0));
        h.run_steps(2);
        let draft = h.state().link_draft.as_ref().unwrap();
        assert_eq!(draft.url, urls[source]);
        assert_eq!(draft.doc, ids[source]);
        if source == 0
            && !reorder
            && let Ok(out) = std::env::var("PDFCRAFT_LINK_SHOT")
        {
            h.run_steps(3);
            h.render().unwrap().save(out).unwrap();
        }
        h.state_mut().link_draft.as_mut().unwrap().url = "https://example.org/changed".into();
        if reorder {
            h.state_mut().views.swap(0, 1);
            h.state_mut().active = Some(0); // Keep the new document active after reordering.
        }
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().link_draft.is_none());
        assert_eq!(h.state().active_ids().unwrap().1, ids[1]);
        assert_eq!(h.state().session.get(ids[source]).unwrap().links[0].action, LinkAction::Uri("https://example.org/changed".into()));
        assert_eq!(h.state().session.get(ids[1 - source]).unwrap().links[0].action, LinkAction::Uri(urls[1 - source].into()));
        h.state_mut().session.undo(ids[source]).unwrap();
        assert_eq!(h.state().session.get(ids[source]).unwrap().links[0].action, LinkAction::Uri(urls[source].into()));
        // Drawing a new link uses the same document-bound draft, including after reordering.
        let view = h.state().views.iter().position(|view| view.id == ids[source]).unwrap();
        h.state_mut().views[view].links.open_new = Some((0, [20.0, 70.0, 100.0, 90.0]));
        h.run_steps(2);
        assert_eq!(h.state().link_draft.as_ref().unwrap().doc, ids[source]);
        h.state_mut().link_draft.as_mut().unwrap().url = "https://example.org/added".into();
        h.get_by_label("OK").click();
        h.run_steps(3);
        let links = &h.state().session.get(ids[source]).unwrap().links;
        assert_eq!(links.len(), 2);
        assert_eq!(links[1].action, LinkAction::Uri("https://example.org/added".into()));
        assert_eq!(h.state().session.get(ids[1 - source]).unwrap().links.len(), 1);
    }
}

#[test]
fn an_edit_for_a_closed_document_does_not_fall_back_to_the_active_tab() {
    use pdfcraft_engine::{Edit, LinkAction, LinkStyle};
    let bytes = Session::new().create_from_text("t", "A fixture.").unwrap();
    let mut app = PdfCraftApp::new();
    app.open_bytes("closed.pdf", None, bytes.as_ref().clone()).unwrap();
    let closed = app.views[0].id;
    app.open_bytes("active.pdf", None, bytes.as_ref().clone()).unwrap();
    let active = app.views[1].id;
    app.session.close(closed);
    app.views.remove(0);
    app.active = Some(0);
    assert!(!app.apply_edit_to(
        closed,
        Edit::AddLink {
            page: 0,
            rect: [20.0, 20.0, 100.0, 50.0],
            action: LinkAction::Uri("https://example.org/wrong".into()),
            style: LinkStyle::default(),
        }
    ));
    assert!(app.session.get(active).unwrap().info.links.is_empty());
    assert!(app.session.get(active).unwrap().can_undo().is_none());
}
