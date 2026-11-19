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
const MAX_VECTOR_PATH_COMMANDS: usize = 100_000;
const MAX_VECTOR_XML_BYTES: usize = 64 * 1024 * 1024;

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn n(v: f32) -> String {
    if v == 0.0 { "0".into() } else { format!("{v:.4}") }
}

fn point(v: anicraft_geom::Vec2) -> Result<String> {
    let x = v.x * TWIPS;
    let y = v.y * TWIPS;
    if !(x.is_finite() && y.is_finite()) {
        return Err(EngineError::Format("vector coordinates exceed the finite XFL range".into()));
    }
    Ok(format!("{} {}", n(x), n(y)))
}

fn edge_data(path: &Path) -> Result<String> {
    let mut out = String::new();
    let mut current = None;
    let mut start = None;
    let mut emitted_edges = 0usize;
    for (command_index, cmd) in path.cmds.iter().enumerate() {
        if command_index >= MAX_VECTOR_PATH_COMMANDS {
            return Err(EngineError::Format("vector path exceeds the 100,000-command safety limit".into()));
        }
        match *cmd {
            PathCmd::MoveTo(p) => {
                if !p.is_finite() { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                out.push('!');
                out.push_str(&point(p)?);
                current = Some(p);
                start = Some(p);
            }
            PathCmd::LineTo(p) => {
                if current.is_none() { return Err(EngineError::Format("vector path has a line before its first move".into())); }
                if !p.is_finite() { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                if emitted_edges >= MAX_VECTOR_PATH_COMMANDS { return Err(EngineError::Format("vector path exceeds the 100,000-edge safety limit".into())); }
                out.push('|');
                out.push_str(&point(p)?);
                emitted_edges += 1;
                current = Some(p);
            }
            PathCmd::QuadTo(c, p) => {
                if current.is_none() { return Err(EngineError::Format("vector path has a curve before its first move".into())); }
                if !(c.is_finite() && p.is_finite()) { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                if emitted_edges >= MAX_VECTOR_PATH_COMMANDS { return Err(EngineError::Format("vector path exceeds the 100,000-edge safety limit".into())); }
                out.push(']');
                out.push_str(&point(c)?);
                out.push(' ');
                out.push_str(&point(p)?);
                emitted_edges += 1;
                current = Some(p);
            }
            PathCmd::CubicTo(a, b, p) => {
                let Some(from) = current else { return Err(EngineError::Format("vector path has a curve before its first move".into())); };
                if !(a.is_finite() && b.is_finite() && p.is_finite()) { return Err(EngineError::Format("vector path contains a non-finite point".into())); }
                const STEPS: usize = 12;
                if emitted_edges.saturating_add(STEPS) > MAX_VECTOR_PATH_COMMANDS {
                    return Err(EngineError::Format("vector path exceeds the 100,000-edge safety limit after cubic conversion".into()));
                }
                for i in 1..=STEPS {
                    let t = i as f32 / STEPS as f32;
                    let u = 1.0 - t;
                    let q = from * (u*u*u) + a * (3.0*u*u*t) + b * (3.0*u*t*t) + p * (t*t*t);
                    out.push('|');
                    out.push_str(&point(q)?);
                }
                emitted_edges += STEPS;
                current = Some(p);
            }
            PathCmd::Close => {
                if let (Some(at), Some(first)) = (current, start) {
                    if (at - first).length() > 0.0001 {
                        if emitted_edges >= MAX_VECTOR_PATH_COMMANDS { return Err(EngineError::Format("vector path exceeds the 100,000-edge safety limit".into())); }
                        out.push('|');
                        out.push_str(&point(first)?);
                        emitted_edges += 1;
                    }
                }
                current = start;
            }
        }
    }
    if out.is_empty() { return Err(EngineError::Format("empty vector path".into())); }
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
        if !line.width.is_finite() || line.width <= 0.0 {
            return Err(EngineError::Format("native FLA export requires a finite, positive stroke width".into()));
        }
        if !line.style.miter_limit.is_finite()
            || !(anicraft_doc::stroke_style::MIN_MITER_LIMIT..=anicraft_doc::stroke_style::MAX_MITER_LIMIT).contains(&line.style.miter_limit)
        {
            return Err(EngineError::Format("native FLA export requires a finite miter limit between 1 and 100".into()));
        }
        let weight_twips = line.width * TWIPS;
        if !weight_twips.is_finite() {
            return Err(EngineError::Format("vector stroke width exceeds the finite XFL range".into()));
        }
        stroke_index = 1;
        let c = line.color.sanitized();
        let caps = match line.style.cap { anicraft_doc::Cap::Butt => "none", anicraft_doc::Cap::Round => "round", anicraft_doc::Cap::Square => "square" };
        let joins = match line.style.join { anicraft_doc::Join::Miter => "miter", anicraft_doc::Join::Round => "round", anicraft_doc::Join::Bevel => "bevel" };
        stroke = format!("<strokes><StrokeStyle index=\"1\"><SolidStroke scaleMode=\"normal\" weight=\"{}\" caps=\"{}\" joints=\"{}\" miterLimit=\"{}\"><fill><SolidColor color=\"{}\" alpha=\"{}\"/></fill></SolidStroke></StrokeStyle></strokes>", n(weight_twips), caps, joins, n(line.style.miter_limit), c.to_hex(), n(c.a));
    }
    if fill_index == 0 && stroke_index == 0 { return Ok(String::new()); }
    let mut matrix = String::new();
    if transform != anicraft_geom::Affine::IDENTITY {
        if !transform.is_finite() { return Err(EngineError::Format("element transform is not finite".into())); }
        let [a,b,c,d,tx,ty] = transform.m;
        let tx = tx * TWIPS;
        let ty = ty * TWIPS;
        if !(tx.is_finite() && ty.is_finite()) {
            return Err(EngineError::Format("element translation exceeds the finite XFL range".into()));
        }
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

fn push_bounded(out: &mut String, part: &str) -> Result<()> {
    push_bounded_to(out, part, MAX_VECTOR_XML_BYTES)
}

fn push_bounded_to(out: &mut String, part: &str, limit: usize) -> Result<()> {
    if out.len().saturating_add(part.len()) > limit {
        return Err(EngineError::Format("native FLA document XML exceeds the 64 MB safety limit".into()));
    }
    out.push_str(part);
    Ok(())
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
            let end_frame = key.start.checked_add(key.duration.max(1))
                .ok_or_else(|| EngineError::Format("native FLA keyframe range overflows".into()))?;
            if end_frame > MAX_VECTOR_FRAMES {
                return Err(EngineError::Format(format!("native FLA export is limited to {MAX_VECTOR_FRAMES} frames per scene")));
            }
            if key.tween.is_some() || key.motion.is_some() || !key.hints.is_empty() || key.sound.is_some() || key.script.is_some() || !key.pose.is_empty() {
                return Err(EngineError::Format(format!("native FLA export does not yet support tweens, frame scripts, sounds, shape hints, or armature poses (layer '{}', frame {})", layer.name, key.start)));
            }
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
                let shape = shape_xml(shape, element.transform)?;
                push_bounded(&mut elements, &shape)?;
            }
            push_bounded(&mut elements, "</elements>")?;
            let label = if key.label.is_empty() { String::new() } else { format!(" name=\"{}\"", xml(&key.label)) };
            let frame_xml = format!("<DOMFrame index=\"{}\" duration=\"{}\" keyMode=\"9728\"{label}>{elements}</DOMFrame>", key.start, key.duration.max(1));
            push_bounded(&mut frames, &frame_xml)?;
        }
        push_bounded(&mut frames, "</frames>")?;
        let visible = if layer.visible { "true" } else { "false" };
        let locked = if layer.locked { "true" } else { "false" };
        let layer_xml = format!("<DOMLayer name=\"{}\" color=\"{}\" visible=\"{}\" locked=\"{}\">{frames}</DOMLayer>", xml(&layer.name), layer.color.to_hex(), visible, locked);
        push_bounded(&mut out, &layer_xml)?;
    }
    push_bounded(&mut out, "</layers>")?;
    Ok(out)
}

/// Build an editable XFL package. The current supported subset is explicit and fails closed.
pub(crate) fn export(doc: &Document, path: &FsPath) -> Result<(Vec<u8>, u32, usize)> {
    if path.extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case("fla")) {
        return Err(EngineError::Param("FLA export needs a .fla file name".into()));
    }
    if !doc.library.is_empty() || !doc.bitmaps.is_empty() || !doc.sounds.is_empty() || !doc.document_class.is_empty() {
        return Err(EngineError::Format("native FLA export does not yet support library symbols, bitmaps, sounds, or document scripts".into()));
    }
    let stage = doc.stage;
    if stage.width == 0 || stage.height == 0
        || stage.width > anicraft_doc::MAX_STAGE_EDGE
        || stage.height > anicraft_doc::MAX_STAGE_EDGE
    {
        return Err(EngineError::Format("native FLA export requires stage dimensions between 1 and 16,384 pixels".into()));
    }
    if !stage.fps.is_finite() || !(1.0..=240.0).contains(&stage.fps) {
        return Err(EngineError::Format("native FLA export requires a finite frame rate between 1 and 240 fps".into()));
    }
    let frames = doc.frame_count();
    if frames > MAX_VECTOR_FRAMES {
        return Err(EngineError::Param(format!("native FLA export is limited to {MAX_VECTOR_FRAMES} frames")));
    }
    let mut shapes = 0usize;
    let mut timelines = String::from("<timelines>");
    for scene in &doc.scenes {
        let layers = timeline_xml(&scene.timeline, &mut shapes)?;
        let timeline = format!("<DOMTimeline name=\"{}\">{layers}</DOMTimeline>", xml(&scene.name));
        push_bounded(&mut timelines, &timeline)?;
    }
    push_bounded(&mut timelines, "</timelines>")?;
    if stage.background.a != 1.0 {
        return Err(EngineError::Format("native FLA export does not yet support a translucent stage background".into()));
    }
    let xml_doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><DOMDocument xmlns=\"http://ns.adobe.com/xfl/2008/\" width=\"{}\" height=\"{}\" frameRate=\"{}\" backgroundColor=\"{}\" currentTimeline=\"1\" xflVersion=\"2.971\" creatorInfo=\"AniCraft\"><folders/>{timelines}</DOMDocument>", stage.width, stage.height, n(stage.fps), stage.background.to_hex());
    if xml_doc.len() > MAX_VECTOR_XML_BYTES {
        return Err(EngineError::Format("native FLA document XML exceeds the 64 MB safety limit".into()));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("AniCraft");
    let files = vec![
        // XFL packages put the uncompressed mimetype entry first in the ZIP.
        (PathBuf::from("mimetype"), b"application/vnd.adobe.xfl".to_vec()),
        (PathBuf::from("DOMDocument.xml"), xml_doc.into_bytes()),
        (PathBuf::from(format!("{stem}.xfl")), b"PROXY-CS5".to_vec()),
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
    fn xml_budget_is_enforced_before_appending() {
        let mut xml = String::from("1234");
        push_bounded_to(&mut xml, "56", 5).expect_err("append must exceed the configured budget");
        assert_eq!(xml, "1234", "failed append must not partially mutate the output");
        push_bounded_to(&mut xml, "5", 5).expect("append within budget should succeed");
        assert_eq!(xml, "12345");
    }

    #[test]
    fn keyframe_ranges_beyond_fla_limit_are_rejected() {
        let mut doc = Document::default();
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        key.start = MAX_VECTOR_FRAMES;
        key.duration = 2;
        let mut shapes = 0;
        let error = timeline_xml(&doc.scenes[0].timeline, &mut shapes)
            .expect_err("keyframe ranges beyond the export limit must be rejected");
        assert!(error.to_string().contains("limited to 10,000 frames"));
    }

    #[test]
    fn invalid_stage_dimensions_are_rejected() {
        let mut doc = Document::default();
        doc.stage.width = 0;
        let error = export(&doc, FsPath::new("invalid.fla"))
            .expect_err("zero-width stages cannot be exported");
        assert!(error.to_string().contains("stage dimensions"));
    }

    #[test]
    fn invalid_stage_frame_rate_is_rejected() {
        let mut doc = Document::default();
        doc.stage.fps = f32::INFINITY;
        let error = export(&doc, FsPath::new("invalid.fla"))
            .expect_err("non-finite frame rates cannot be exported");
        assert!(error.to_string().contains("frame rate"));
    }

    #[test]
    fn cubic_paths_are_approximated_as_bounded_editable_edges() {
        let path = Path {
            cmds: vec![
                PathCmd::MoveTo(anicraft_geom::Vec2::new(0.0, 0.0)),
                PathCmd::CubicTo(
                    anicraft_geom::Vec2::new(0.0, 10.0),
                    anicraft_geom::Vec2::new(10.0, 10.0),
                    anicraft_geom::Vec2::new(10.0, 0.0),
                ),
            ],
        };
        let edges = edge_data(&path).expect("a finite cubic path should export");
        assert!(edges.starts_with("!0 0"));
        assert_eq!(edges.matches('|').count(), 12, "one cubic is approximated by twelve editable line segments");
    }

    #[test]
    fn vector_path_command_limit_is_enforced() {
        let mut cmds = Vec::with_capacity(MAX_VECTOR_PATH_COMMANDS + 1);
        cmds.push(PathCmd::MoveTo(anicraft_geom::Vec2::new(0.0, 0.0)));
        cmds.extend((0..MAX_VECTOR_PATH_COMMANDS).map(|_| {
            PathCmd::LineTo(anicraft_geom::Vec2::new(1.0, 1.0))
        }));
        let error = edge_data(&Path { cmds }).expect_err("oversized paths must be rejected");
        assert!(error.to_string().contains("100,000-command safety limit"));
    }

    #[test]
    fn stroke_width_and_transform_translation_use_xfl_twips() {
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 10.0, 10.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0x336699)),
        );
        shape.stroke = Some(anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0));
        let xml = shape_xml(
            &shape,
            anicraft_geom::Affine::translate(3.0, 4.0),
        ).expect("finite shape should export");
        assert!(xml.contains("weight=\"40\""), "stroke width must be converted from pixels to twips: {xml}");
        assert!(xml.contains("tx=\"60\" ty=\"80\""), "matrix translation must be converted from pixels to twips: {xml}");
    }

    #[test]
    fn stroke_width_that_overflows_twips_is_rejected() {
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 10.0, 10.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0x336699)),
        );
        let mut stroke = anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0);
        stroke.width = f32::MAX;
        shape.stroke = Some(stroke);
        assert!(shape_xml(&shape, anicraft_geom::Affine::IDENTITY)
            .expect_err("overflowing twip width must be rejected")
            .to_string().contains("finite XFL range"));
    }

    #[test]
    fn invalid_stroke_dimensions_are_rejected() {
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 10.0, 10.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0x336699)),
        );
        let mut stroke = anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0);
        stroke.width = f32::NAN;
        shape.stroke = Some(stroke);
        assert!(shape_xml(&shape, anicraft_geom::Affine::IDENTITY)
            .expect_err("non-finite stroke widths must be rejected")
            .to_string().contains("positive stroke width"));
    }

    #[test]
    fn converted_cubic_edges_obey_the_output_complexity_limit() {
        let mut cmds = vec![PathCmd::MoveTo(anicraft_geom::Vec2::new(0.0, 0.0))];
        cmds.extend((0..(MAX_VECTOR_PATH_COMMANDS / 12 + 1)).map(|_| {
            PathCmd::CubicTo(
                anicraft_geom::Vec2::new(1.0, 2.0),
                anicraft_geom::Vec2::new(2.0, 2.0),
                anicraft_geom::Vec2::new(3.0, 0.0),
            )
        }));
        let error = edge_data(&Path { cmds }).expect_err("cubic expansion must remain bounded");
        assert!(error.to_string().contains("100,000-edge safety limit"));
    }

    #[test]
    fn coordinates_that_overflow_twips_are_rejected() {
        let path = Path { cmds: vec![PathCmd::MoveTo(anicraft_geom::Vec2::new(f32::MAX, 0.0))] };
        let error = edge_data(&path).expect_err("finite source coordinates can overflow XFL twips");
        assert!(error.to_string().contains("finite XFL range"));
    }

    #[test]
    fn invalid_miter_limits_are_rejected() {
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 10.0, 10.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0x336699)),
        );
        let mut stroke = anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0);
        stroke.style.miter_limit = f32::NAN;
        shape.stroke = Some(stroke);
        assert!(shape_xml(&shape, anicraft_geom::Affine::IDENTITY)
            .expect_err("non-finite miter limits must be rejected")
            .to_string().contains("miter limit"));
    }

    #[test]
    fn line_and_quadratic_paths_without_a_move_are_rejected() {
        let line = Path { cmds: vec![PathCmd::LineTo(anicraft_geom::Vec2::new(1.0, 1.0))] };
        assert!(edge_data(&line).expect_err("line needs a starting point").to_string().contains("line before its first move"));
        let quad = Path { cmds: vec![PathCmd::QuadTo(
            anicraft_geom::Vec2::new(1.0, 1.0),
            anicraft_geom::Vec2::new(2.0, 2.0),
        )] };
        assert!(edge_data(&quad).expect_err("quadratic needs a starting point").to_string().contains("curve before its first move"));
    }

    #[test]
    fn cubic_path_without_a_move_is_rejected() {
        let path = Path {
            cmds: vec![PathCmd::CubicTo(
                anicraft_geom::Vec2::new(1.0, 1.0),
                anicraft_geom::Vec2::new(2.0, 2.0),
                anicraft_geom::Vec2::new(3.0, 3.0),
            )],
        };
        assert!(edge_data(&path).is_err());
    }

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
