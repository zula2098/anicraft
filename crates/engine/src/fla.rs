//! First native-vector slice of the XFL/FLA exporter.
//!
//! It writes editable paths, solid/gradient/bitmap fills, library symbols, bitmap and sound assets, and keyframe timing.
//! Unsupported document features are rejected rather than silently flattened or discarded.

use crate::{EngineError, Result};
use anicraft_doc::{Document, ElementKind, LayerKind, Paint};
use anicraft_doc::sound::{SoundEffect, SoundFormat};
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

fn ng(v: f32) -> String {
    if v == 0.0 { "0".into() } else { format!("{v:.8}") }
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


fn transform_xml(transform: anicraft_geom::Affine) -> Result<String> {
    if transform == anicraft_geom::Affine::IDENTITY {
        return Ok(String::new());
    }
    if !transform.is_finite() {
        return Err(EngineError::Format("element transform is not finite".into()));
    }
    let [a, b, c, d, tx, ty] = transform.m;
    Ok(format!("<matrix><Matrix a=\"{}\" b=\"{}\" c=\"{}\" d=\"{}\" tx=\"{}\" ty=\"{}\"/></matrix>", n(a), n(b), n(c), n(d), n(tx), n(ty)))
}

fn shape_xml(shape: &anicraft_doc::Shape, transform: anicraft_geom::Affine, bitmap_names: &std::collections::HashMap<anicraft_doc::BitmapId, String>) -> Result<String> {
    let mut fill = String::new();
    let mut stroke = String::new();
    let mut fill_index = 0;
    let mut stroke_index = 0;
    if shape.fill_rule != anicraft_geom::FillRule::NonZero || shape.merge || shape.warp.is_some() {
        return Err(EngineError::Format("native FLA export does not yet support even-odd fills, merged shapes, or warped artwork".into()));
    }
    if let Some(paint) = &shape.fill {
        fill_index = 1;
        let definition = match paint {
            Paint::Solid(color) => {
                let c = color.sanitized();
                format!("<SolidColor color=\"{}\" alpha=\"{}\"/>", c.to_hex(), n(c.a))
            }
            Paint::Bitmap { bitmap, repeat, matrix } => {
                let name = bitmap_names.get(bitmap).ok_or_else(|| EngineError::Format(format!("shape references missing bitmap {}", bitmap.0)))?;
                if !matrix.is_finite() {
                    return Err(EngineError::Format("bitmap fill transform is not finite".into()));
                }
                let [a,b,c,d,tx,ty] = matrix.m;
                let clipped = if *repeat { "false" } else { "true" };
                format!("<BitmapFill bitmapPath=\"{}\" bitmapIsClipped=\"{}\"><matrix><Matrix a=\"{}\" b=\"{}\" c=\"{}\" d=\"{}\" tx=\"{}\" ty=\"{}\"/></matrix></BitmapFill>", xml(name), clipped, n(a * TWIPS), n(b * TWIPS), n(c * TWIPS), n(d * TWIPS), n(tx), n(ty))
            }
            Paint::Linear { stops, spread, linear_rgb, .. } | Paint::Radial { stops, spread, linear_rgb, .. } => {
                if stops.is_empty() || stops.len() > anicraft_doc::MAX_GRADIENT_STOPS {
                    return Err(EngineError::Format("gradient fills need between 1 and 15 colour stops".into()));
                }
                let frame = paint.frame(None).ok_or_else(|| EngineError::Format("gradient transform is unavailable".into()))?;
                if !frame.is_finite() {
                    return Err(EngineError::Format("gradient transform is not finite".into()));
                }
                let [a, b, c, d, tx, ty] = frame.m;
                let matrix = format!("<matrix><Matrix a=\"{}\" b=\"{}\" c=\"{}\" d=\"{}\" tx=\"{}\" ty=\"{}\"/></matrix>", ng(a / 819.2), ng(b / 819.2), ng(c / 819.2), ng(d / 819.2), n(tx), n(ty));
                let mut entries = String::new();
                for stop in stops {
                    if !stop.offset.is_finite() {
                        return Err(EngineError::Format("gradient stop positions must be finite".into()));
                    }
                    let color = stop.color.sanitized();
                    push_bounded(&mut entries, &format!("<GradientEntry color=\"{}\" alpha=\"{}\" ratio=\"{}\"/>", color.to_hex(), n(color.a), n(stop.offset.clamp(0.0, 1.0))))?;
                }
                let spread_attr = if *spread == anicraft_doc::Spread::Pad { String::new() } else { format!(" spreadMethod=\"{}\"", spread.key()) };
                let interpolation = if *linear_rgb { " interpolationMethod=\"linearRGB\"" } else { "" };
                match paint {
                    Paint::Linear { .. } => format!("<LinearGradient{spread_attr}{interpolation}>{matrix}{entries}</LinearGradient>"),
                    Paint::Radial { focal, .. } => format!("<RadialGradient{spread_attr}{interpolation} focalPointRatio=\"{}\">{matrix}{entries}</RadialGradient>", n(*focal)),
                    _ => return Err(EngineError::Format("invalid gradient paint".into())),
                }
            }
        };
        fill = format!("<fills><FillStyle index=\"1\">{definition}</FillStyle></fills>");
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

fn safe_library_path(name: &str) -> Result<String> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\')
        || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(EngineError::Format(format!("library path '{name}' cannot be represented safely in an XFL package")));
    }
    Ok(name.to_string())
}

fn sound_library_name(doc: &Document, sound: &anicraft_doc::Sound) -> Result<String> {
    let extension = match sound.format {
        SoundFormat::Wav => "wav",
        SoundFormat::Ogg => "ogg",
        SoundFormat::Mp3 => "mp3",
    };
    let entry = doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Sound(sound.id)));
    let mut path = PathBuf::from(entry);
    path.set_extension(extension);
    safe_library_path(&path.to_string_lossy())
}

pub(crate) fn sound_media_xml(doc: &Document) -> Result<(String, Vec<(PathBuf, Vec<u8>)>)> {
    let mut media = String::new();
    let mut files = Vec::with_capacity(doc.sounds.len());
    for sound in &doc.sounds {
        if sound.data.is_empty() {
            return Err(EngineError::Format(format!("sound '{}' has no audio data", sound.name)));
        }
        let name = sound_library_name(doc, sound)?;
        let id = format!("00000000-0000-0000-0000-{:012}", sound.id.0 % 1_000_000_000_000);
        let rate_khz = ((sound.rate + 500) / 1000).max(1);
        let channels = if sound.channels == 1 { "Mono" } else { "Stereo" };
        let format = format!("{rate_khz}kHz 16bit {channels}");
        push_bounded(&mut media, &format!(
            "<DOMSoundItem name=\"{}\" itemID=\"{}\" sourceExternalFilepath=\"./LIBRARY/{}\" href=\"{}\" format=\"{}\"/>",
            xml(&name), id, xml(&name), xml(&name), xml(&format)
        ))?;
        files.push((PathBuf::from("LIBRARY").join(&name), sound.data.as_ref().clone()));
    }
    Ok((media, files))
}

pub(crate) fn frame_sound_xml(key: &anicraft_doc::Keyframe, doc: &Document) -> Result<(String, String)> {
    let Some(frame_sound) = &key.sound else { return Ok((String::new(), String::new())) };
    let sound = doc.sound(frame_sound.sound).ok_or_else(|| {
        EngineError::Format(format!("keyframe references missing sound {}", frame_sound.sound.0))
    })?;
    let name = sound_library_name(doc, sound)?;
    let mut attrs = format!(" soundName=\"{}\" soundSync=\"{}\"", xml(&name), frame_sound.sync.key());
    if frame_sound.looping {
        attrs.push_str(" soundLoopMode=\"loop\"");
    } else if frame_sound.repeat > 1 {
        attrs.push_str(&format!(" soundLoopMode=\"repeat\" soundLoop=\"{}\"", frame_sound.repeat));
    }
    let mut envelope = String::new();
    match &frame_sound.effect {
        SoundEffect::None => {}
        SoundEffect::Custom(points) => {
            attrs.push_str(" soundEffect=\"custom\"");
            if !points.is_empty() {
                envelope.push_str("<SoundEnvelope>");
                for point in anicraft_doc::sound::clean_points(points) {
                    let mark = (point.at * 44_100.0).round().clamp(0.0, u32::MAX as f64) as u32;
                    let left = (f64::from(point.left) * 32_768.0).round().clamp(0.0, 32_768.0) as u32;
                    let right = (f64::from(point.right) * 32_768.0).round().clamp(0.0, 32_768.0) as u32;
                    envelope.push_str(&format!("<SoundEnvelopePoint mark44=\"{mark}\" level0=\"{left}\" level1=\"{right}\"/>"));
                }
                envelope.push_str("</SoundEnvelope>");
            }
        }
        preset => attrs.push_str(&format!(" soundEffect=\"{}\"", preset.key())),
    }
    Ok((attrs, envelope))
}

fn color_effect_xml(effect: anicraft_doc::ColorEffect) -> String {
    let body = match effect.sanitized() {
        anicraft_doc::ColorEffect::None => return String::new(),
        anicraft_doc::ColorEffect::Alpha(alpha) => format!("<Color alphaMultiplier=\"{}\"/>", n(alpha)),
        anicraft_doc::ColorEffect::Brightness(brightness) => format!("<Color brightness=\"{}\"/>", n(brightness)),
        anicraft_doc::ColorEffect::Tint { color, amount } => format!("<Color tintMultiplier=\"{}\" tintColor=\"{}\"/>", n(amount), color.to_hex()),
        anicraft_doc::ColorEffect::Advanced(transform) => format!(
            "<Color redMultiplier=\"{}\" greenMultiplier=\"{}\" blueMultiplier=\"{}\" alphaMultiplier=\"{}\" redOffset=\"{}\" greenOffset=\"{}\" blueOffset=\"{}\" alphaOffset=\"{}\"/>",
            n(transform.mul[0]), n(transform.mul[1]), n(transform.mul[2]), n(transform.mul[3]),
            n(transform.add[0] * 255.0), n(transform.add[1] * 255.0), n(transform.add[2] * 255.0), n(transform.add[3] * 255.0)
        ),
    };
    format!("<color>{body}</color>")
}

fn frame_script_xml(script: Option<&anicraft_doc::FrameScript>) -> Result<String> {
    let Some(script) = script else { return Ok(String::new()) };
    match script {
        anicraft_doc::FrameScript::Source(source) if source.language == anicraft_doc::Language::ActionScript3 => {
            source.check().map_err(|error| EngineError::Format(format!("frame script cannot be exported: {error}")))?;
            Ok(format!("<Actionscript><script>{}</script></Actionscript>", xml(&source.text)))
        }
        anicraft_doc::FrameScript::Source(_) => Err(EngineError::Format("native FLA export currently supports ActionScript 3.0 source frame scripts only".into())),
        anicraft_doc::FrameScript::Actions(_) => Err(EngineError::Format("native FLA export does not yet serialize AniCraft action-list frame scripts".into())),
        anicraft_doc::FrameScript::Plugin(_) => Err(EngineError::Format("native FLA export does not yet serialize plug-in frame scripts".into())),
    }
}

fn timeline_xml(timeline: &anicraft_doc::Timeline, doc: &Document, total_shapes: &mut usize, symbol_names: &std::collections::HashMap<anicraft_doc::SymbolId, String>, bitmap_names: &std::collections::HashMap<anicraft_doc::BitmapId, String>) -> Result<String> {
    let mut out = String::from("<layers>");
    let layer_indices: std::collections::HashMap<_, _> = timeline.layers.iter().enumerate().map(|(index, layer)| (layer.id, index)).collect();
    // Keep folders, masks and guides in the list: parentLayerIndex counts every layer in XFL.
    for layer in &timeline.layers {
        let layer_type = match layer.kind {
            LayerKind::Normal => "",
            LayerKind::Mask => " layerType=\"mask\"",
            LayerKind::Guide => " layerType=\"guide\"",
            LayerKind::Folder => " layerType=\"folder\"",
            other => return Err(EngineError::Format(format!("native FLA export does not yet support {:?} layers", other))),
        };
        let parent = match layer.parent {
            Some(id) => {
                let index = layer_indices.get(&id).ok_or_else(|| EngineError::Format(format!("layer '{}' has a missing parent", layer.name)))?;
                format!(" parentLayerIndex=\"{}\"", index)
            }
            None => String::new(),
        };
        if layer.opacity != 1.0 || layer.depth != 0.0 || layer.attached || !layer.depth_keys.is_empty() {
            return Err(EngineError::Format(format!("native FLA export does not yet support translucent or depth-managed layer '{}'", layer.name)));
        }
        let visible = if layer.visible { "true" } else { "false" };
        let locked = if layer.locked { "true" } else { "false" };
        if layer.kind == LayerKind::Folder {
            let folder = format!("<DOMLayer name=\"{}\" color=\"{}\" visible=\"{}\" locked=\"{}\"{layer_type}{parent}/>", xml(&layer.name), layer.color.to_hex(), visible, locked);
            push_bounded(&mut out, &folder)?;
            continue;
        }
        let mut frames = String::from("<frames>");
        for key in &layer.keyframes {
            let end_frame = key.start.checked_add(key.duration.max(1))
                .ok_or_else(|| EngineError::Format("native FLA keyframe range overflows".into()))?;
            if end_frame > MAX_VECTOR_FRAMES {
                return Err(EngineError::Format(format!("native FLA export is limited to {MAX_VECTOR_FRAMES} frames per scene")));
            }
            if key.tween.is_some() || key.motion.is_some() || !key.hints.is_empty() || !key.pose.is_empty() {
                return Err(EngineError::Format(format!("native FLA export does not yet support tweens, frame scripts, shape hints, or armature poses (layer '{}', frame {})", layer.name, key.start)));
            }
            let mut elements = String::from("<elements>");
            for element in &key.elements {
                match &element.kind {
                    ElementKind::Shape(shape) => {
                        if element.color != anicraft_doc::ColorEffect::default() {
                            return Err(EngineError::Format("native FLA export does not yet apply colour effects to shapes".into()));
                        }
                        *total_shapes += 1;
                        if *total_shapes > MAX_VECTOR_SHAPES {
                            return Err(EngineError::Format("native FLA export exceeds the 100,000-shape safety limit".into()));
                        }
                        let shape = shape_xml(shape, element.transform, bitmap_names)?;
                        push_bounded(&mut elements, &shape)?;
                    }
                    ElementKind::Text(text) => {
                        if element.color != anicraft_doc::ColorEffect::default() {
                            return Err(EngineError::Format("native FLA export does not yet apply colour effects to text outlines".into()));
                        }
                        if text.field.is_some() || !text.filters.is_empty() {
                            return Err(EngineError::Format(format!("text element '{}' has a dynamic field or filters that cannot yet be represented as editable vector paths", element.name)));
                        }
                        for (color, _url, outline) in text.fill_paths() {
                            if outline.cmds.is_empty() {
                                continue;
                            }
                            *total_shapes += 1;
                            if *total_shapes > MAX_VECTOR_SHAPES {
                                return Err(EngineError::Format("native FLA export exceeds the 100,000-shape safety limit".into()));
                            }
                            let outlines = anicraft_doc::Shape {
                                path: outline,
                                fill: Some(Paint::Solid(color)),
                                stroke: None,
                                fill_rule: anicraft_geom::FillRule::NonZero,
                                merge: false,
                                warp: None,
                            };
                            let shape = shape_xml(&outlines, element.transform, bitmap_names)?;
                            push_bounded(&mut elements, &shape)?;
                        }
                    }
                    ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters, events, .. } => {
                        if *blend != anicraft_doc::BlendMode::Normal || !filters.is_empty() || !events.is_empty() {
                            return Err(EngineError::Format(format!("symbol instance '{}' has a blend mode, filter, or event that cannot yet be serialized", element.name)));
                        }
                        let name = symbol_names.get(symbol).ok_or_else(|| EngineError::Format(format!("symbol instance '{}' references missing library item {}", element.name, symbol.0)))?;
                        let kind = doc.symbol(*symbol).map(|s| s.kind).ok_or_else(|| EngineError::Format(format!("symbol {} is missing", symbol.0)))?;
                        let symbol_type = match kind {
                            anicraft_doc::SymbolKind::Graphic => " symbolType=\"graphic\"",
                            anicraft_doc::SymbolKind::MovieClip => "",
                            anicraft_doc::SymbolKind::Button => " symbolType=\"button\"",
                        };
                        let loop_value = match loop_mode {
                            anicraft_doc::LoopMode::Loop => "loop",
                            anicraft_doc::LoopMode::PlayOnce => "play once",
                            anicraft_doc::LoopMode::SingleFrame => "single frame",
                        };
                        let label = if element.name.is_empty() { String::new() } else { format!(" name=\"{}\"", xml(&element.name)) };
                        let matrix = transform_xml(element.transform)?;
                        let color = color_effect_xml(element.color);
                        let instance = format!("<DOMSymbolInstance libraryItemName=\"{}\"{symbol_type} loop=\"{loop_value}\" firstFrame=\"{}\"{label}>{matrix}{color}</DOMSymbolInstance>", xml(name), first_frame);
                        push_bounded(&mut elements, &instance)?;
                    }
                    _ => {
                        return Err(EngineError::Format(format!("native FLA export does not yet support element '{}' of type {:?}", element.name, element.kind)));
                    }
                }
            }
            push_bounded(&mut elements, "</elements>")?;
            let label = if key.label.is_empty() { String::new() } else { format!(" name=\"{}\"", xml(&key.label)) };
            let (sound_attrs, sound_envelope) = frame_sound_xml(key, doc)?;
            let script = frame_script_xml(key.script.as_ref())?;
            let frame_xml = format!("<DOMFrame index=\"{}\" duration=\"{}\" keyMode=\"9728\"{label}{sound_attrs}>{sound_envelope}{script}{elements}</DOMFrame>", key.start, key.duration.max(1));
            push_bounded(&mut frames, &frame_xml)?;
        }
        push_bounded(&mut frames, "</frames>")?;
        let layer_xml = format!("<DOMLayer name=\"{}\" color=\"{}\" visible=\"{}\" locked=\"{}\"{layer_type}{parent}>{frames}</DOMLayer>", xml(&layer.name), layer.color.to_hex(), visible, locked);
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
    if !doc.document_class.is_empty() {
        return Err(EngineError::Format("native FLA export does not yet support document class scripts".into()));
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
    let mut symbol_names = std::collections::HashMap::new();
    for symbol in &doc.library {
        let name = safe_library_path(&doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Symbol(symbol.id))) )?;
        symbol_names.insert(symbol.id, name);
    }
    let mut bitmap_names = std::collections::HashMap::new();
    for bitmap in &doc.bitmaps {
        let name = safe_library_path(&doc.entry_path(anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Bitmap(bitmap.id))) )?;
        bitmap_names.insert(bitmap.id, name);
    }
    let mut symbols_xml = String::new();
    let mut symbol_files = Vec::with_capacity(doc.library.len());
    for symbol in &doc.library {
        let name = symbol_names.get(&symbol.id).ok_or_else(|| EngineError::Format("symbol name was not indexed".into()))?;
        let layers = timeline_xml(&symbol.timeline, doc, &mut shapes, &symbol_names, &bitmap_names)?;
        let symbol_type = match symbol.kind {
            anicraft_doc::SymbolKind::Graphic => " symbolType=\"graphic\"",
            anicraft_doc::SymbolKind::MovieClip => "",
            anicraft_doc::SymbolKind::Button => " symbolType=\"button\"",
        };
        let mut attrs = String::new();
        if let Some(linkage) = &symbol.linkage {
            attrs.push_str(&format!(" linkageExportForAS=\"true\" linkageClassName=\"{}\"", xml(&linkage.class)));
            if !linkage.base.is_empty() {
                attrs.push_str(&format!(" linkageBaseClass=\"{}\"", xml(&linkage.base)));
            }
        }
        let scale_grid = symbol.scale_grid.map(|g| format!(" scaleGridLeft=\"{}\" scaleGridRight=\"{}\" scaleGridTop=\"{}\" scaleGridBottom=\"{}\"", n(g.left * TWIPS), n(g.right * TWIPS), n(g.top * TWIPS), n(g.bottom * TWIPS))).unwrap_or_default();
        let item_id = format!("00000000-0000-0000-0000-{:012}", symbol.id.0 % 1_000_000_000_000);
        let item = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><DOMSymbolItem xmlns=\"http://ns.adobe.com/xfl/2008/\" name=\"{}\" itemID=\"{}\"{symbol_type}{attrs}{scale_grid}><timeline><DOMTimeline name=\"{}\">{layers}</DOMTimeline></timeline></DOMSymbolItem>", xml(name), item_id, xml(&symbol.name));
        if item.len() > MAX_VECTOR_XML_BYTES {
            return Err(EngineError::Format(format!("symbol '{}' XML exceeds the 64 MB safety limit", symbol.name)));
        }
        let href = format!("{name}.xml");
        push_bounded(&mut symbols_xml, &format!("<Include href=\"{}\"/>", xml(&href)))?;
        symbol_files.push((PathBuf::from("LIBRARY").join(&href), item.into_bytes()));
    }
    let mut bitmap_media = String::new();
    let mut bitmap_files = Vec::with_capacity(doc.bitmaps.len());
    for bitmap in &doc.bitmaps {
        if bitmap.png.is_empty() {
            return Err(EngineError::Format(format!("bitmap '{}' has no PNG data", bitmap.name)));
        }
        let name = bitmap_names.get(&bitmap.id).ok_or_else(|| EngineError::Format("bitmap name was not indexed".into()))?;
        let id = format!("00000000-0000-0000-0000-{:012}", bitmap.id.0 % 1_000_000_000_000);
        let smoothing = if bitmap.smoothing { "true" } else { "false" };
        push_bounded(&mut bitmap_media, &format!("<DOMBitmapItem name=\"{}\" itemID=\"{}\" sourceExternalFilepath=\"./LIBRARY/{}\" href=\"{}\" allowSmoothing=\"{}\"/>", xml(name), id, xml(name), xml(name), smoothing))?;
        bitmap_files.push((PathBuf::from("LIBRARY").join(name), bitmap.png.as_ref().clone()));
    }
    let mut timelines = String::from("<timelines>");
    for scene in &doc.scenes {
        let layers = timeline_xml(&scene.timeline, doc, &mut shapes, &symbol_names, &bitmap_names)?;
        let timeline = format!("<DOMTimeline name=\"{}\">{layers}</DOMTimeline>", xml(&scene.name));
        push_bounded(&mut timelines, &timeline)?;
    }
    push_bounded(&mut timelines, "</timelines>")?;
    if stage.background.a != 1.0 {
        return Err(EngineError::Format("native FLA export does not yet support a translucent stage background".into()));
    }
    let (media, mut sound_files) = sound_media_xml(doc)?;
    let mut media_xml = bitmap_media;
    push_bounded(&mut media_xml, &media)?;
    let xml_doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><DOMDocument xmlns=\"http://ns.adobe.com/xfl/2008/\" width=\"{}\" height=\"{}\" frameRate=\"{}\" backgroundColor=\"{}\" currentTimeline=\"1\" xflVersion=\"2.971\" creatorInfo=\"AniCraft\"><folders/><media>{media_xml}</media><symbols>{symbols_xml}</symbols>{timelines}</DOMDocument>", stage.width, stage.height, n(stage.fps), stage.background.to_hex());
    if xml_doc.len() > MAX_VECTOR_XML_BYTES {
        return Err(EngineError::Format("native FLA document XML exceeds the 64 MB safety limit".into()));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("AniCraft");
    let mut files = vec![
        // XFL packages put the uncompressed mimetype entry first in the ZIP.
        (PathBuf::from("mimetype"), b"application/vnd.adobe.xfl".to_vec()),
        (PathBuf::from("DOMDocument.xml"), xml_doc.into_bytes()),
        (PathBuf::from(format!("{stem}.xfl")), b"PROXY-CS5".to_vec()),
    ];
    files.append(&mut bitmap_files);
    files.append(&mut symbol_files);
    files.append(&mut sound_files);
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
        let error = timeline_xml(&doc.scenes[0].timeline, &doc, &mut shapes, &std::collections::HashMap::new(), &std::collections::HashMap::new())
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
    fn stroke_width_and_transform_translation_use_pixels() {
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 10.0, 10.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0x336699)),
        );
        shape.stroke = Some(anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0));
        let xml = shape_xml(
            &shape,
            anicraft_geom::Affine::translate(3.0, 4.0),
            &std::collections::HashMap::new(),
        ).expect("finite shape should export");
        assert!(xml.contains("weight=\"2\""), "stroke width is expressed in pixels: {xml}");
        assert!(xml.contains("tx=\"3\" ty=\"4\""), "matrix translation is expressed in pixels: {xml}");
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
        assert!(shape_xml(&shape, anicraft_geom::Affine::IDENTITY, &std::collections::HashMap::new())
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
        assert!(shape_xml(&shape, anicraft_geom::Affine::IDENTITY, &std::collections::HashMap::new())
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
    fn editable_vector_package_round_trips_gradient_fills() {
        let mut doc = Document::default();
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        let paint = Paint::Linear {
            start: anicraft_geom::Vec2::new(-30.0, 0.0),
            end: anicraft_geom::Vec2::new(30.0, 0.0),
            stops: vec![
                anicraft_doc::GradientStop { offset: 0.0, color: anicraft_doc::Rgba::hex(0x112233) },
                anicraft_doc::GradientStop { offset: 0.4, color: anicraft_doc::Rgba::hex(0x445566).with_alpha(0.7) },
                anicraft_doc::GradientStop { offset: 1.0, color: anicraft_doc::Rgba::hex(0x778899) },
            ],
            spread: anicraft_doc::Spread::Reflect,
            linear_rgb: true,
        };
        key.elements.push(Arc::new(Element {
            id: ElementId(8001),
            name: "gradient".into(),
            transform: anicraft_geom::Affine::IDENTITY,
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 60.0, 20.0), paint)),
        }));
        let (bytes, _, _) = export(&doc, FsPath::new("gradient-roundtrip.fla")).expect("gradient package should export");
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open the gradient package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        let element = &opened.timeline(0).layers[0].keyframe_at(0).expect("main keyframe").elements[0];
        let ElementKind::Shape(shape) = &element.kind else { panic!("gradient remains a shape"); };
        let Some(Paint::Linear { stops, spread, linear_rgb, start, end }) = &shape.fill else { panic!("linear gradient should round-trip"); };
        assert_eq!(*spread, anicraft_doc::Spread::Reflect);
        assert!(*linear_rgb);
        assert_eq!(stops.len(), 3);
        assert!((stops[1].offset - 0.4).abs() < 0.001);
        assert!((start.x + 30.0).abs() < 0.01 && (end.x - 30.0).abs() < 0.01);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_keeps_actionscript_frame_scripts() {
        let mut doc = Document::default();
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        key.script = Some(anicraft_doc::FrameScript::Source(Arc::new(anicraft_doc::SourceScript::new(
            anicraft_doc::Language::ActionScript3,
            r#"trace("hello");"#,
        ))));
        let (bytes, _, _) = export(&doc, FsPath::new("script-roundtrip.fla")).expect("script package should export");
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open the script package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        let script = opened.timeline(0).layers[0].keyframe_at(0).and_then(|key| key.script.as_ref()).expect("frame script should remain");
        let Some(source) = script.source() else { panic!("source script should remain source text"); };
        assert_eq!(source.text, r#"trace("hello");"#);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_round_trips_mask_layers_and_parent_links() {
        let mut doc = Document::default();
        let timeline = doc.timeline_mut(0);
        let mask_id = timeline.layers[0].id;
        Arc::make_mut(&mut timeline.layers[0]).kind = LayerKind::Mask;
        let mut child = anicraft_doc::Layer::new(anicraft_doc::LayerId(9902), "Masked artwork");
        child.parent = Some(mask_id);
        child.keyframe_mut(0).expect("child layer has a first keyframe").elements.push(Arc::new(Element {
            id: ElementId(9903),
            name: "masked shape".into(),
            transform: anicraft_geom::Affine::IDENTITY,
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 16.0, 16.0), Paint::Solid(anicraft_doc::Rgba::hex(0x336699)))),
        }));
        timeline.layers.push(Arc::new(child));
        let (bytes, _, _) = export(&doc, FsPath::new("mask-roundtrip.fla")).expect("mask export should succeed");
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open mask package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        let layers = &opened.timeline(0).layers;
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].kind, LayerKind::Mask);
        assert_eq!(layers[1].parent, Some(layers[0].id));
        assert_eq!(layers[1].keyframe_at(0).map(|key| key.elements.len()), Some(1));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_round_trips_symbols_and_bitmap_fills() {
        let mut doc = Document::default();
        let symbol_id = anicraft_doc::SymbolId(700);
        let mut symbol_timeline = anicraft_doc::Timeline::default();
        let symbol_layer = Arc::make_mut(&mut symbol_timeline.layers[0]);
        let symbol_key = symbol_layer.keyframe_mut(0).expect("symbol layer has a first keyframe");
        symbol_key.elements.push(Arc::new(Element {
            id: ElementId(701),
            name: "inside symbol".into(),
            transform: anicraft_geom::Affine::IDENTITY,
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(
                Path::rect(0.0, 0.0, 12.0, 8.0),
                Paint::Solid(anicraft_doc::Rgba::hex(0x22AA77)),
            )),
        }));
        doc.library.push(Arc::new(anicraft_doc::Symbol {
            id: symbol_id,
            name: "Badge".into(),
            kind: anicraft_doc::SymbolKind::Graphic,
            linkage: None,
            scale_grid: None,
            scale_grid_kept: None,
            timeline: symbol_timeline,
        }));

        let bitmap = anicraft_doc::Bitmap::from_rgba(
            anicraft_doc::BitmapId(702),
            "checker.png",
            2,
            2,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
        ).expect("test bitmap should encode");
        let bitmap_id = bitmap.id;
        doc.bitmaps.push(Arc::new(bitmap));

        let main_layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let main_key = main_layer.keyframe_mut(0).expect("main layer has a first keyframe");
        main_key.elements.push(Arc::new(Element {
            id: ElementId(703),
            name: "symbol instance".into(),
            transform: anicraft_geom::Affine::translate(24.0, 16.0),
            color: anicraft_doc::ColorEffect::Alpha(0.5),
            kind: ElementKind::Symbol {
                symbol: symbol_id,
                loop_mode: anicraft_doc::LoopMode::SingleFrame,
                first_frame: 0,
                blend: anicraft_doc::BlendMode::Normal,
                filters: Vec::new(),
                events: Vec::new(),
                pivot: None,
            },
        }));
        let mut bitmap_shape = Shape::filled(
            Path::rect(0.0, 0.0, 20.0, 20.0),
            Paint::Bitmap {
                bitmap: bitmap_id,
                repeat: false,
                matrix: anicraft_geom::Affine::scale(10.0, 10.0),
            },
        );
        bitmap_shape.stroke = None;
        main_key.elements.push(Arc::new(Element {
            id: ElementId(704),
            name: "bitmap fill".into(),
            transform: anicraft_geom::Affine::translate(4.0, 6.0),
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(bitmap_shape),
        }));

        let (bytes, _, _) = export(&doc, FsPath::new("library-roundtrip.fla")).expect("library export should succeed");
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open the library package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        assert_eq!(opened.library.len(), 1);
        assert_eq!(opened.bitmaps.len(), 1);
        assert_eq!(opened.library[0].name, "Badge");
        let elements = &opened.timeline(0).layers[0].keyframe_at(0).expect("main keyframe").elements;
        assert_eq!(elements.len(), 2);
        assert!(matches!(elements[0].kind, ElementKind::Symbol { .. }));
        assert_eq!(elements[0].color, anicraft_doc::ColorEffect::Alpha(0.5));
        let ElementKind::Shape(shape) = &elements[1].kind else { panic!("bitmap fill should remain a shape"); };
        assert!(matches!(shape.fill, Some(Paint::Bitmap { .. })));
        let symbol_shape = &opened.library[0].timeline.layers[0].keyframe_at(0).expect("symbol keyframe").elements[0];
        assert!(matches!(symbol_shape.kind, ElementKind::Shape(_)));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_keeps_embedded_sound_and_frame_settings() {
        let mut doc = Document::default();
        let sound = anicraft_doc::Sound::from_data(
            anicraft_doc::SoundId(77),
            "tone.mp3",
            include_bytes!("../../audio/tests/data/tone.mp3").to_vec(),
        ).expect("the test MP3 should decode");
        doc.sounds.push(Arc::new(sound));
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        key.sound = Some(anicraft_doc::FrameSound {
            sound: anicraft_doc::SoundId(77),
            sync: anicraft_doc::SoundSync::Stream,
            repeat: 1,
            looping: true,
            effect: anicraft_doc::SoundEffect::Custom(vec![
                anicraft_doc::EnvelopePoint::new(0.0, 0.0, 1.0),
                anicraft_doc::EnvelopePoint::new(0.25, 1.0, 0.0),
            ]),
        });

        let (bytes, _, _) = export(&doc, FsPath::new("sound-roundtrip.fla")).expect("sound package should export");
        let (opened, report) = anicraft_xfl::import_fla(&bytes).expect("XFL importer should open the sound package");
        assert!(report.skipped.is_empty(), "unexpected import skips: {:?}", report.skipped);
        assert_eq!(opened.sounds.len(), 1);
        assert_eq!(opened.sounds[0].format, SoundFormat::Mp3);
        let imported = opened.timeline(0).layers.first().and_then(|layer| layer.keyframe_at(0))
            .and_then(|key| key.sound.as_ref()).expect("frame sound should round-trip");
        assert_eq!(imported.sync, anicraft_doc::SoundSync::Stream);
        assert!(imported.looping);
        assert!(matches!(imported.effect, SoundEffect::Custom(ref points) if points.len() == 2));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn editable_vector_package_round_trips_through_xfl_importer() {
        let mut doc = Document::default();
        let layer = Arc::make_mut(&mut doc.timeline_mut(0).layers[0]);
        let key = layer.keyframe_mut(0).expect("new layer has a first keyframe");
        key.duration = 4;
        let mut shape = Shape::filled(
            Path::rect(0.0, 0.0, 40.0, 24.0),
            Paint::Solid(anicraft_doc::Rgba::hex(0xD9452B)),
        );
        shape.stroke = Some(anicraft_doc::Stroke::new(anicraft_doc::Rgba::hex(0x112233), 2.0));
        key.elements.push(Arc::new(Element {
            id: ElementId(9001),
            name: "editable rectangle".into(),
            transform: anicraft_geom::Affine { m: [1.5, 0.25, -0.5, 0.75, 12.0, 8.0] },
            color: anicraft_doc::ColorEffect::default(),
            kind: ElementKind::Shape(shape),
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
        let element = imported.and_then(|k| k.elements.first()).expect("round-tripped shape should exist");
        let expected_transform = [1.5, 0.25, -0.5, 0.75, 12.0, 8.0];
        for (actual, expected) in element.transform.m.iter().zip(expected_transform) {
            assert!((actual - expected).abs() < 0.0001, "transform coefficient {actual} differs from {expected}");
        }
        let ElementKind::Shape(imported_shape) = &element.kind else {
            panic!("round-tripped element should remain a vector shape");
        };
        assert_eq!(imported_shape.stroke.as_ref().map(|stroke| stroke.width), Some(2.0));
    }
}
