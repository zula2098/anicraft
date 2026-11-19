//! First native-vector slice of the XFL/FLA exporter.
//!
//! It writes editable paths, solid fills/strokes, layer visibility, and keyframe timing.
//! Unsupported document features are rejected rather than silently flattened or discarded.

use crate::{EngineError, Result};
use anicraft_doc::{Document, ElementKind, LayerKind, Paint};
use anicraft_geom::PathCmd;
use anicraft_geom::Path;
use std::path::{Path as FsPath, PathBuf};

const TWIPS: f32 = 20.0;
const MAX_VECTOR_FRAMES: u32 = 10_000;
const MAX_VECTOR_SHAPES: usize = 100_000;

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn n(v: f32) -> String {
    if v == 0.0 { "0".into() } else { format!("{v:.4}") }
}

fn point(v: anicraft_geom::Vec2) -> String {
    format!("{} {}", n(v.x * TWIPS), n(v.y * TWIPS))
}

fn edge_data(path: &Path) -> Result<String> {
    let mut out = String::new();
    let mut current = None;
    let mut start = None;
    let mut cubic_segments = 0usize;
    for cmd in &path.cmds {
        match *cmd {
            PathCmd::MoveTo(p) => {
                if !p.is_finite() { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                out.push('!');
                out.push_str(&point(p));
                current = Some(p);
                start = Some(p);
            }
            PathCmd::LineTo(p) => {
                if !p.is_finite() { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                out.push('|');
                out.push_str(&point(p));
                current = Some(p);
            }
            PathCmd::QuadTo(c, p) => {
                if !(c.is_finite() && p.is_finite()) { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                out.push(']');
                out.push_str(&point(c));
                out.push(' ');
                out.push_str(&point(p));
                current = Some(p);
            }
            PathCmd::CubicTo(a, b, p) => {
                let Some(from) = current else { return Err(EngineError::Format("vector path has a curve before its first move".into())); };
                if !(a.is_finite() && b.is_finite() && p.is_finite()) { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                // XFL's edge wire format uses quadratic segments. Approximate a cubic by short
                // editable line segments, rather than rasterizing the curve.
                const STEPS: usize = 12;
                for i in 1..=STEPS {
                    let t = i as f32 / STEPS as f32;
                    let u = 1.0 - t;
                    let q = from * (u*u*u) + a * (3.0*u*u*t) + b * (3.0*u*t*t) + p * (t*t*t);
                    out.push('|');
                    out.push_str(&point(q));
                }
                cubic_segments += STEPS;
                current = Some(p);
            }
            PathCmd::Close => {
                if let (Some(at), Some(first)) = (current, start) {
                        if (at - first).length() > 0.0001 {
                            out.push('|');
                            out.push_str(&point(first));
                        }
                }
                current = start;
            }
        }
    }
    if out.is_empty() { return Err(EngineError::Format("empty vector path".into())); }
    if cubic_segments > 100_000 { return Err(EngineError::Format("vector path is too complex for FLA export".into())); }
    Ok(out)
}

fn shape_xml(shape: &anicraft_doc::Shape, transform: anicraft_geom::Affine) -> Result<String> {
    let mut fill = String::new();
    let mut stroke = String::new();
    let mut fill_index = 0;
    let mut stroke_index = 0;
    if shape.fill_rule != anicraft_geom::FillRule::NonZero || shape.merge || shape.warp.is_some() {
        return Err(EngineError::Format("native FLA export does not yet support even-odd fills, merged shapes, or warped artwork".into()));
    }
    if let Some(paint) = &shape.fill {
        let Paint::Solid(color) = paint else {
            return Err(EngineError::Format("native FLA export currently supports solid shape fills only; gradients and bitmap fills are not yet supported".into()));
        };
        fill_index = 1;
        let c = color.sanitized();
        fill = format!("<fills><FillStyle index=\"1\"><SolidColor color=\"{}\" alpha=\"{}\"/></FillStyle></fills>", c.to_hex(), n(c.a));
    }
    if let Some(line) = &shape.stroke {
        if line.profile.is_some() || line.style.dash.is_some() {
            return Err(EngineError::Format("native FLA export currently supports uniform solid strokes only; variable-width and dashed strokes are not yet supported".into()));
        }
        stroke_index = 1;
        let c = line.color.sanitized();
        let caps = match line.style.cap { anicraft_doc::Cap::Butt => "none", anicraft_doc::Cap::Round => "round", anicraft_doc::Cap::Square => "square" };
        let joins = match line.style.join { anicraft_doc::Join::Miter => "miter", anicraft_doc::Join::Round => "round", anicraft_doc::Join::Bevel => "bevel" };
        stroke = format!("<strokes><StrokeStyle index=\"1\"><SolidStroke scaleMode=\"normal\" weight=\"{}\" caps=\"{}\" joints=\"{}\" miterLimit=\"{}\"><fill><SolidColor color=\"{}\" alpha=\"{}\"/></fill></SolidStroke></StrokeStyle></strokes>", n(line.width), caps, joins, n(line.style.miter_limit), c.to_hex(), n(c.a));
    }
    if fill_index == 0 && stroke_index == 0 { return Ok(String::new()); }
    let mut matrix = String::new();
    if transform != anicraft_geom::Affine::IDENTITY {
        if !transform.is_finite() { return Err(EngineError::Format("element transform is not finite".into())); }
        let [a,b,c,d,tx,ty] = transform.m;
        matrix = format!("<matrix><Matrix a=\"{}\" b=\"{}\" c=\"{}\" d=\"{}\" tx=\"{}\" ty=\"{}\"/></matrix>", n(a), n(b), n(c), n(d), n(tx), n(ty));
    }
    let edges = edge_data(&shape.path)?;
    let attrs = match (fill_index, stroke_index) {
        (1,1) => " fillStyle1=\"1\" strokeStyle=\"1\"",
        (1,0) => " fillStyle1=\"1\"",
        (0,1) => " strokeStyle=\"1\"",
        _ => "",
    };
    Ok(format!("<DOMShape>{matrix}{fill}{stroke}<edges><Edge{attrs} edges=\"{edges}\"/></edges></DOMShape>"))
}

fn timeline_xml(timeline: &anicraft_doc::Timeline, total_shapes: &mut usize) -> Result<String> {
    let mut out = String::from("<layers>");
    // Both the document model and XFL store the frontmost layer first.
    for layer in &timeline.layers {
        if layer.kind == LayerKind::Folder { continue; }
        if layer.kind != LayerKind::Normal {
            return Err(EngineError::Format(format!("native FLA export does not yet support {:?} layers", layer.kind)));
        }
        if layer.parent.is_some() || layer.opacity != 1.0 || layer.depth != 0.0 || layer.attached || !layer.depth_keys.is_empty() {
            return Err(EngineError::Format(format!("native FLA export does not yet support parented, translucent, or depth-managed layer '{}'", layer.name)));
        }
        let mut frames = String::from("<frames>");
        for key in &layer.keyframes {
            let mut elements = String::from("<elements>");
            for element in &key.elements {
                let ElementKind::Shape(shape) = &element.kind else {
                    return Err(EngineError::Format(format!("native FLA export currently supports vector shapes only; element '{}' is not a plain shape", element.name)));
                };
                if element.color != anicraft_doc::ColorEffect::default() {
                    return Err(EngineError::Format("native FLA export does not yet support element colour effects".into()));
                }
                *total_shapes += 1;
                if *total_shapes > MAX_VECTOR_SHAPES {
                    return Err(EngineError::Format("native FLA export exceeds the 100,000-shape safety limit".into()));
                }
                elements.push_str(&shape_xml(shape, element.transform)?);
            }
            elements.push_str("</elements>");
            let label = if key.label.is_empty() { String::new() } else { format!(" name=\"{}\"", xml(&key.label)) };
            frames.push_str(&format!("<DOMFrame index=\"{}\" duration=\"{}\" keyMode=\"9728\"{label}>{elements}</DOMFrame>", key.start, key.duration.max(1)));
        }
        frames.push_str("</frames>");
        let visible = if layer.visible { "true" } else { "false" };
        let locked = if layer.locked { "true" } else { "false" };
        out.push_str(&format!("<DOMLayer name=\"{}\" color=\"{}\" visible=\"{}\" locked=\"{}\">{frames}</DOMLayer>", xml(&layer.name), layer.color.to_hex(), visible, locked));
    }
    out.push_str("</layers>");
    Ok(out)
}

/// Build an editable XFL package. The current supported subset is explicit and fails closed.
pub(crate) fn export(doc: &Document, path: &FsPath) -> Result<(Vec<u8>, u32, usize)> {
    if path.extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case("fla")) {
        return Err(EngineError::Param("FLA export needs a .fla file name".into()));
    }
    let frames = doc.frame_count();
    if frames > MAX_VECTOR_FRAMES {
        return Err(EngineError::Param(format!("native FLA export is limited to {MAX_VECTOR_FRAMES} frames")));
    }
    let mut shapes = 0usize;
    let mut timelines = String::from("<timelines>");
    for scene in &doc.scenes {
        let layers = timeline_xml(&scene.timeline, &mut shapes)?;
        timelines.push_str(&format!("<DOMTimeline name=\"{}\">{layers}</DOMTimeline>", xml(&scene.name)));
    }
    timelines.push_str("</timelines>");
    let stage = doc.stage;
    if stage.background.a != 1.0 {
        return Err(EngineError::Format("native FLA export does not yet support a translucent stage background".into()));
    }
    let xml_doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><DOMDocument xmlns=\"http://ns.adobe.com/xfl/2008/\" width=\"{}\" height=\"{}\" frameRate=\"{}\" backgroundColor=\"{}\" currentTimeline=\"1\" xflVersion=\"2.971\" creatorInfo=\"AniCraft\"><folders/>{timelines}</DOMDocument>", stage.width, stage.height, n(stage.fps), stage.background.to_hex());
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("AniCraft");
    let files = vec![
        (PathBuf::from("DOMDocument.xml"), xml_doc.into_bytes()),
        (PathBuf::from(format!("{stem}.xfl")), b"PROXY-CS5".to_vec()),
        (PathBuf::from("mimetype"), b"application/vnd.adobe.xfl".to_vec()),
    ];
    let bytes = crate::publish::zip(&files).map_err(EngineError::Format)?;
    Ok((bytes, frames, shapes))
}


#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::{Element, ElementId, Shape};
    use anicraft_geom::Path;
    use std::sync::Arc;

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_round_trips_through_xfl_importer() {
        let mut doc = Document::default();
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        key.duration = 4;
        key.elements.push(Arc::new(Element {
            id: ElementId(9001),
            name: "editable rectangle".into(),
            transform: anicraft_geom::Affine::translate(12.0, 8.0),
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(
                Path::rect(0.0, 0.0, 40.0, 24.0),
                Paint::Solid(anicraft_doc::Rgba::hex(0xD9452B)),
            )),
        }));

        let (bytes, frames, shapes) = export(&doc, FsPath::new("roundtrip.fla")).expect("export should succeed");
        assert_eq!(frames, 4);
        assert_eq!(shapes, 1);
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open the package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        assert_eq!(opened.stage.width, doc.stage.width);
        assert_eq!(opened.stage.height, doc.stage.height);
        assert_eq!(opened.frame_count(), 4);
        let imported = opened.timeline(0).layers.first().and_then(|l| l.keyframe_at(0));
        assert_eq!(imported.map(|k| k.elements.len()), Some(1));
    }
}
