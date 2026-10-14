//! The area of the selection, shown in the Info panel: the bounding box's area (the sheet a print
//! needs: width × height) and the area the selected closed shapes enclose (what is printed or cut,
//! with holes left out and overlaps counted once).

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, PathData};

use super::*;

/// Square metres in one square point (1 pt = 1/72 in = 0.0254/72 m).
pub const SQUARE_METRES_PER_SQUARE_POINT: f64 = (0.0254 / 72.0) * (0.0254 / 72.0);

/// Paths are measured up to this many; a larger selection reports only its bounding box, so a
/// selection of a whole map can't stall the panel.
pub const MAX_SHAPES: usize = 20_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "selection.area",
        "Area",
        [],
        None,
        "{ids?} → {boxArea, shapeArea, shapes, width, height, unit: \"pt²\", squareMetres: {box, shape}}: boxArea is the selection's bounding box (width × height, strokes left out); shapeArea the area its closed paths and compound paths enclose, united (holes left out, overlaps counted once). Images, placed files, symbols and text count their bounds; open paths, guides and clipping paths count nothing, and a clip group counts its clipping path. shapeArea is null past 20,000 shapes",
        has_selection,
        area
    )]
}

/// Area of `ids` in `doc`, in square points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    /// Bounding box width and height.
    pub width: f64,
    pub height: f64,
    /// `width × height`.
    pub bounds: f64,
    /// Enclosed area of the closed shapes, united; `None` past [`MAX_SHAPES`].
    pub shape: Option<f64>,
    /// The shapes measured.
    pub shapes: usize,
}

/// Measure `ids` (see [`Area`]); `None` when they have no extent.
pub fn measure(doc: &Document, ids: &[NodeId]) -> Option<Area> {
    let b = doc.bounds_of(ids, false)?;
    let (width, height) = (b.width(), b.height());
    if !(width.is_finite() && height.is_finite()) {
        return None;
    }
    let mut shapes: Vec<(PathData, FillRule)> = vec![];
    let mut over = false;
    for n in ids.iter().filter_map(|id| doc.node(*id)) {
        collect(n, &mut shapes, &mut over);
        if over {
            break;
        }
    }
    let shape = (!over).then(|| {
        let refs: Vec<(&PathData, FillRule)> = shapes.iter().map(|(p, r)| (p, *r)).collect();
        match refs.as_slice() {
            [] => 0.0,
            [(p, r)] => vectorcraft_pathops::area(p, *r),
            _ => vectorcraft_pathops::area(&vectorcraft_pathops::unite_all(&refs), FillRule::NonZero),
        }
    });
    Some(Area { width, height, bounds: width * height, shape: shape.filter(|a| a.is_finite()), shapes: shapes.len() })
}

/// Only the closed subpaths of `p`: an open path encloses nothing.
fn closed_only(p: &PathData) -> Option<PathData> {
    let subpaths: Vec<_> = p.subpaths.iter().filter(|sp| sp.closed && sp.anchors.len() > 2).cloned().collect();
    (!subpaths.is_empty()).then(|| PathData::new(subpaths))
}

fn push(out: &mut Vec<(PathData, FillRule)>, over: &mut bool, shape: (PathData, FillRule)) {
    if out.len() >= MAX_SHAPES {
        *over = true;
    } else {
        out.push(shape);
    }
}

fn collect(n: &Node, out: &mut Vec<(PathData, FillRule)>, over: &mut bool) {
    if *over || !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { path, rule, clipping, guide, .. } => {
            if !clipping
                && !guide
                && let Some(p) = closed_only(path)
            {
                push(out, over, (p, *rule));
            }
        }
        NodeKind::Compound { children, rule } => {
            // The compound's subpaths fill together, so its holes stay holes.
            let subpaths: Vec<_> = children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect();
            if let Some(p) = closed_only(&PathData::new(subpaths)) {
                push(out, over, (p, *rule));
            }
        }
        NodeKind::Group { children, clip: true } => {
            // What shows of a clip group lies inside its clipping path.
            if let Some(p) = children.first().and_then(|c| c.path_data()).and_then(closed_only) {
                push(out, over, (p, FillRule::NonZero));
            }
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } => {
            for c in n.children().into_iter().flatten() {
                collect(c, out, over);
            }
        }
        _ => {
            if let Some(b) = n.geometric_bounds()
                && b.width() > 0.0
                && b.height() > 0.0
            {
                push(out, over, (vectorcraft_geom::shapes::rectangle(b), FillRule::NonZero));
            }
        }
    }
}

fn area(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "selection.area";
    let ids = checked_ids_or_selection(s, p, C)?;
    let st = s.doc()?;
    // Objects without extent (an empty group) measure nothing.
    let a = measure(&st.doc, &ids).unwrap_or_default();
    Ok(json!({
        "width": a.width,
        "height": a.height,
        "boxArea": a.bounds,
        "shapeArea": a.shape,
        "shapes": a.shapes,
        "unit": "pt²",
        "squareMetres": {
            "box": a.bounds * SQUARE_METRES_PER_SQUARE_POINT,
            "shape": a.shape.map(|v| v * SQUARE_METRES_PER_SQUARE_POINT),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 5000, "height": 5000})).unwrap();
        s
    }

    fn id(v: &Value) -> u64 {
        v["id"].as_u64().unwrap()
    }

    #[test]
    fn a_rectangle_measures_its_box_and_its_shape() {
        let mut s = session();
        let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap();
        let a = s.execute("selection.area", &json!({"ids": [id(&r)]})).unwrap();
        assert!((a["boxArea"].as_f64().unwrap() - 20_000.0).abs() < 1e-6, "{a}");
        assert!((a["shapeArea"].as_f64().unwrap() - 20_000.0).abs() < 1.0, "{a}");
        assert_eq!(a["unit"], "pt²");
    }

    #[test]
    fn an_ellipse_encloses_less_than_its_box() {
        let mut s = session();
        let e = s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        let a = s.execute("selection.area", &json!({"ids": [id(&e)]})).unwrap();
        let shape = a["shapeArea"].as_f64().unwrap();
        // π r² with r = 100, within the Bézier circle's error.
        assert!((shape - std::f64::consts::PI * 10_000.0).abs() < 50.0, "{shape}");
        assert!((a["boxArea"].as_f64().unwrap() - 40_000.0).abs() < 1e-6);
    }

    #[test]
    fn overlapping_shapes_count_once_and_square_metres_follow() {
        let mut s = session();
        // Two 1 m × 1 m squares (2834.6 pt), overlapping by half: 1.5 m² of shape, a 1.5 m × 1 m box.
        let m = 72.0 / 0.0254;
        let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": m, "height": m})).unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": m / 2.0, "y": 0, "width": m, "height": m})).unwrap();
        let r = s.execute("selection.area", &json!({"ids": [id(&a), id(&b)]})).unwrap();
        assert!((r["squareMetres"]["shape"].as_f64().unwrap() - 1.5).abs() < 1e-3, "{r}");
        assert!((r["squareMetres"]["box"].as_f64().unwrap() - 1.5).abs() < 1e-6, "{r}");
        assert_eq!(r["shapes"], 2);
    }

    #[test]
    fn open_paths_enclose_nothing() {
        let mut s = session();
        let l = s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 100, "y2": 100})).unwrap();
        let r = s.execute("selection.area", &json!({"ids": [id(&l)]})).unwrap();
        assert_eq!(r["shapeArea"].as_f64().unwrap(), 0.0, "{r}");
        assert_eq!(r["shapes"], 0);
    }
}
