//! Edit Dimension Text: type the text a dimension shows instead of its measurement (double-click a
//! dimension, or Object › Dimensions › Edit Dimension Text…). `<>` stands for the measurement, so
//! `<> ±0.5` keeps the measured value; an empty field shows the measurement again. OK runs
//! `dimension.update {ids, text}`; the measurement keeps following the measured artwork.
//!
//! Fields: `text`, `__id` (the dimension group) and `__measured` (its measured label).

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Edit Dimension Text.
pub const KIND: &str = "dimensionText";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Edit Dimension Text").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// The dimension group `id` belongs to (itself, or the dimension whose generated art it is).
pub fn dimension_of(app: &VectorcraftApp, id: NodeId) -> Option<NodeId> {
    let doc = &app.session.active()?.doc;
    doc.ancestry(id)?.into_iter().rev().find(|a| doc.node(*a).is_some_and(|n| n.extras().dimension.is_some()))
}

/// Open Edit Dimension Text for dimension group `id`, or the one selected when `id` is `None`.
pub fn open(app: &mut VectorcraftApp, id: Option<NodeId>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let id = id.or_else(|| st.selection.objects.iter().find_map(|o| dimension_of(app, *o))).ok_or_else(|| tl!("Select a dimension").to_string())?;
    let dim = st.doc.node(id).and_then(|n| n.extras().dimension.as_deref()).ok_or_else(|| tl!("Select a dimension").to_string())?;
    let measured = dim.measured_label()?;
    let fields = json!({"text": dim.text.clone().unwrap_or_default(), "__id": id.0, "__measured": measured});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({"dialog": KIND}))
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let measured = d.str("__measured");
    widgets::label_row(ui, tl!("Measured:"), 96.0, |ui| {
        ui.label(egui::RichText::new(&measured).monospace().color(t.text));
    });
    widgets::label_row(ui, tl!("Text:"), 96.0, |ui| {
        form::text(ui, d, "text", 220.0);
    });
    let typed = d.str("text");
    let shown = if typed.trim().is_empty() { measured.clone() } else { typed.trim().replace("<>", &measured) };
    widgets::label_row(ui, tl!("Shows:"), 96.0, |ui| {
        ui.label(egui::RichText::new(shown).monospace().color(t.text));
    });
    ui.label(egui::RichText::new(tl!("<> stands for the measured value. Leave empty to show the measurement.")).small().color(t.text_dim));
    if ui.button(tl!("Use Measured Value")).clicked() {
        d.fields.insert("text".into(), json!(""));
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    // A missing id reaches `dimension.update`, which says to select a dimension.
    let id = d.fields.get("__id").and_then(Value::as_u64).unwrap_or(0);
    run_and_close(app, "dimension.update", json!({"ids": [id], "text": d.str("text")}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::NodeKind;
    use vectorcraft_engine::Session;

    fn shown(app: &VectorcraftApp, id: u64) -> String {
        let n = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        n.children().unwrap().iter().find_map(|c| if let NodeKind::Text(t) = &c.kind { Some(t.plain_text()) } else { None }).unwrap()
    }

    #[test]
    fn opens_on_a_dimension_part_and_types_over_the_measurement() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("dimension.create", json!({"a": [0, 100], "b": [72, 100], "options": {"units": "in", "precision": 0}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        // A click lands on the dimension's generated art: its group is the one edited.
        let part = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap()[0].id;
        assert_eq!(dimension_of(&app, part), Some(NodeId(id)));
        app.run("dimension.editText", json!({"id": id})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.kind.as_str(), d.str("__measured"), d.str("text")), (KIND, "1 in".to_string(), String::new()));
        d.fields.insert("text".into(), json!("<> (25.4 mm)"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(shown(&app, id), "1 in (25.4 mm)");
        // Reopened, it shows the typed text; Use Measured Value empties it again.
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("dimension.editText", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.str("text"), "<> (25.4 mm)");
        d.fields.insert("text".into(), json!(""));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(shown(&app, id), "1 in");
        // Without a dimension selected it says what to select.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("dimension.editText", json!({})).is_err());
    }
}
