use clap::Parser;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use rayon::prelude::*;
use unityfs::assets::AssetManager;
use unityfs::classes::TryFromUnityValue;
use unityfs::classes::mesh::Mesh;
use unityfs::value::UnityValue;
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(value_name = "PATH")]
    inputs: Vec<String>,
    #[arg(short, long)]
    input: Option<String>,
    #[arg(short, long, default_value = "./out")]
    output: String,
    #[arg(short, long)]
    metadata: bool,
    #[arg(long = "name", short = 'n')]
    filter_name: Option<String>,
    #[arg(long = "type", short = 't')]
    filter_type: Option<String>,
    #[arg(long = "by-file", short = 'b', help = "Extract into subdirectories named after each bundle file, without asset class subfolders")]
    by_file: bool,
    #[arg(long = "live2d", short = 'l', help = "Reconstruct and merge Live2D models from extracted assets")]
    live2d: bool,
}
#[derive(Clone, Debug)]
struct ExtractFilter {
    extract_metadata: bool,
    filter_name: Option<String>,
    filter_type: Option<String>,
    by_file: bool,
    live2d: bool,
}
fn extract_bundle_files_in_parallel(files: &[PathBuf], base_output_dir: &Path, filter: &ExtractFilter) {
    let pb = indicatif::ProgressBar::new(files.len() as u64);
    pb.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(50));
    pb.tick();
    files.par_iter().for_each(|file_path| {
        extract_bundle_file(file_path, base_output_dir, filter, &pb);
        pb.inc(1);
    });
    pb.finish();
}
fn collect_files_recursively(dir: &Path, files: &mut Vec<PathBuf>) {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    collect_files_recursively(&entry_path, files);
                } else if entry_path.is_file() && is_unity_bundle(&entry_path) {
                    files.push(entry_path);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to read directory '{}': {}", dir.display(), e);
        }
    }
}
fn main() {
    let args = Args::parse();
    let mut input_paths = args.inputs;
    if let Some(opt_input) = args.input {
        if !input_paths.contains(&opt_input) {
            input_paths.push(opt_input);
        }
    }
    let output_dir = PathBuf::from(&args.output);
    let filter = ExtractFilter {
        extract_metadata: args.metadata,
        filter_name: args.filter_name,
        filter_type: args.filter_type,
        by_file: args.by_file,
        live2d: args.live2d,
    };
    if input_paths.is_empty() {
        run_interactive_mode(&output_dir, &filter);
    } else {
        let mut files_to_extract = Vec::new();
        for path_str in &input_paths {
            let path = Path::new(path_str);
            if !path.exists() {
                eprintln!("Error: Path '{}' does not exist.", path.display());
                continue;
            }
            if path.is_dir() {
                collect_files_recursively(path, &mut files_to_extract);
            } else if path.is_file() {
                files_to_extract.push(path.to_path_buf());
            }
        }
        if files_to_extract.is_empty() {
            println!("No Unity asset bundles found to extract.");
        } else {
            extract_bundle_files_in_parallel(&files_to_extract, &output_dir, &filter);
            if filter.live2d {
                reconstruct_live2d_models(&output_dir);
            }
        }
    }
}
fn clean_drag_drop_path(input: &str) -> String {
    let mut cleaned = input.trim().to_string();
    if cleaned.starts_with('"') && cleaned.ends_with('"') {
        cleaned.remove(0);
        cleaned.pop();
    } else if cleaned.starts_with('\'') && cleaned.ends_with('\'') {
        cleaned.remove(0);
        cleaned.pop();
    }
    cleaned.trim().to_string()
}
#[derive(Debug)]
enum UniquePathResult {
    New(PathBuf),
    Exists(PathBuf),
}
fn get_unique_path(dir: &Path, filename: &str, data: Option<&[u8]>) -> UniquePathResult {
    let base_path = dir.join(filename);
    if !base_path.exists() {
        return UniquePathResult::New(base_path);
    }
    if let Some(bytes) = data {
        if let Ok(metadata) = std::fs::metadata(&base_path) {
            if metadata.len() == bytes.len() as u64 {
                return UniquePathResult::Exists(base_path);
            }
        }
    }
    let stem = base_path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = base_path.extension().unwrap_or_default().to_string_lossy();
    let mut counter = 2;
    loop {
        let new_filename = if extension.is_empty() {
            format!("{} ({})", stem, counter)
        } else {
            format!("{} ({}).{}", stem, counter, extension)
        };
        let new_path = dir.join(new_filename);
        if !new_path.exists() {
            return UniquePathResult::New(new_path);
        }
        if let Some(bytes) = data {
            if let Ok(metadata) = std::fs::metadata(&new_path) {
                if metadata.len() == bytes.len() as u64 {
                    return UniquePathResult::Exists(new_path);
                }
            }
        }
        counter += 1;
    }
}
fn run_interactive_mode(output_dir: &Path, filter: &ExtractFilter) {
    println!("=========================================");
    println!("  Asset Tool CLI - Drag & Drop Extractor ");
    println!("=========================================");
    println!("Please drag & drop a file or folder here, then press Enter to extract.");
    println!("(Or type 'exit' or 'q' to quit)\n");
    loop {
        print!("Path: ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        let cleaned_path_str = clean_drag_drop_path(&input);
        if cleaned_path_str.is_empty() {
            continue;
        }
        if cleaned_path_str.eq_ignore_ascii_case("exit") || cleaned_path_str.eq_ignore_ascii_case("q") {
            println!("Exiting...");
            break;
        }
        let path = Path::new(&cleaned_path_str);
        if !path.exists() {
            eprintln!("Error: Path '{}' does not exist. Please try again.\n", cleaned_path_str);
            continue;
        }
        extract_path(path, output_dir, filter);
        if filter.live2d {
            reconstruct_live2d_models(output_dir);
        }
        println!("\nReady for next input. Drag & drop another file/folder, or 'q' to quit.");
    }
}
fn extract_path(path: &Path, base_output_dir: &Path, filter: &ExtractFilter) {
    if !path.exists() {
        eprintln!("Error: Path '{}' does not exist.", path.display());
        return;
    }
    if path.is_dir() {
        let mut files = Vec::new();
        collect_files_recursively(path, &mut files);
        if files.is_empty() {
            println!("No Unity asset bundles found in directory '{}'.", path.display());
        } else {
            extract_bundle_files_in_parallel(&files, base_output_dir, filter);
        }
    } else if path.is_file() {
        extract_bundle_files_in_parallel(&[path.to_path_buf()], base_output_dir, filter);
    }
}
fn is_unity_bundle(path: &Path) -> bool {
    if let Ok(mut file) = File::open(path) {
        let mut header = [0u8; 8];
        if let Ok(n) = file.read(&mut header) {
            let s = &header[..n];
            return s.starts_with(b"UnityFS") || s.starts_with(b"UnityWeb") || s.starts_with(b"UnityRaw");
        }
    }
    false
}
fn find_and_extract_criware_bytes(
    value: &UnityValue,
    output_dir: &Path,
    base_name: &str,
    parent_key: &str,
    by_file: bool,
    pb: &indicatif::ProgressBar,
) -> bool {
    match value {
        UnityValue::Bytes(bytes) => {
            if bytes.len() > 16 {
                let header = &bytes[..std::cmp::min(16, bytes.len())];
                let mut ext = None;
                if header.starts_with(b"@UTF") {
                    ext = Some("acb");
                } else if header.starts_with(b"AFS2") || header.starts_with(b"AWBP") {
                    ext = Some("awb");
                } else if header.starts_with(b"CPK ") {
                    ext = Some("cpk");
                } else if header.starts_with(b"RIFF") {
                    ext = Some("wav");
                } else if header.starts_with(b"OggS") {
                    ext = Some("ogg");
                } else if header.len() >= 8 && &header[4..8] == b"ftyp" {
                    ext = Some("m4a");
                }
                if let Some(extension) = ext {
                    let filename = if parent_key.is_empty() {
                        format!("{}.{}", base_name, extension)
                    } else {
                        format!("{}_{}.{}", base_name, parent_key, extension)
                    };
                    let cri_dir = if by_file {
                        output_dir.to_path_buf()
                    } else {
                        output_dir.join("CriWare")
                    };
                    let _ = std::fs::create_dir_all(&cri_dir);
                    return match get_unique_path(&cri_dir, &filename, Some(bytes)) {
                        UniquePathResult::New(dest) => {
                            if let Err(e) = std::fs::write(&dest, bytes) {
                                pb.println(format!("    Failed to write CriWare asset '{}': {}", dest.display(), e));
                                false
                            } else {
                                true
                            }
                        }
                        UniquePathResult::Exists(_) => true,
                    };
                }
            }
            false
        }
        UnityValue::Array(arr) => {
            if !arr.is_empty() && arr.len() > 16 {
                let mut bytes = Vec::with_capacity(arr.len());
                let mut is_byte_array = true;
                for item in arr {
                    match item {
                        UnityValue::UInt8(b) => bytes.push(*b),
                        UnityValue::Int8(b) => bytes.push(*b as u8),
                        UnityValue::UInt16(b) => bytes.push(*b as u8),
                        UnityValue::Int16(b) => bytes.push(*b as u8),
                        UnityValue::UInt32(b) => bytes.push(*b as u8),
                        UnityValue::Int32(b) => bytes.push(*b as u8),
                        UnityValue::UInt64(b) => bytes.push(*b as u8),
                        UnityValue::Int64(b) => bytes.push(*b as u8),
                        _ => {
                            is_byte_array = false;
                            break;
                        }
                    }
                }
                if is_byte_array {
                    return find_and_extract_criware_bytes(&UnityValue::Bytes(bytes), output_dir, base_name, parent_key, by_file, pb);
                }
            }
            let mut extracted = false;
            for (idx, item) in arr.iter().enumerate() {
                let item_key = if parent_key.is_empty() {
                    idx.to_string()
                } else {
                    format!("{}_{}", parent_key, idx)
                };
                if find_and_extract_criware_bytes(item, output_dir, base_name, &item_key, by_file, pb) {
                    extracted = true;
                }
            }
            extracted
        }
        UnityValue::Map(map) => {
            let mut extracted = false;
            for (k, v) in map {
                let item_key = if parent_key.is_empty() {
                    k.clone()
                } else {
                    format!("{}_{}", parent_key, k)
                };
                if find_and_extract_criware_bytes(v, output_dir, base_name, &item_key, by_file, pb) {
                    extracted = true;
                }
            }
            extracted
        }
        _ => false,
    }
}
struct PosePartData {
    id: String,
    group_index: i32,
    link: Vec<String>,
}
fn get_mono_behaviour_class_name(
    asset_manager: &AssetManager,
    asset_name: &str,
    m_script_pptr: &UnityValue,
) -> Option<String> {
    if let UnityValue::PPtr { file_id, path_id } = m_script_pptr {
        if let Ok(script_val) = asset_manager.read_object_value(asset_name, *file_id, *path_id) {
            if let Some(UnityValue::String(class_name)) = script_val.get("m_ClassName") {
                return Some(class_name.clone());
            }
        }
    }
    None
}
fn resolve_class_name(
    value: &UnityValue,
    asset_manager: &AssetManager,
    asset_name: &str,
) -> Option<String> {
    if let Some(script_pptr) = value.get("m_Script") {
        if let Some(class_name) = get_mono_behaviour_class_name(asset_manager, asset_name, script_pptr) {
            return Some(class_name);
        }
    }
    if value.get("ParameterIds").is_some() && value.get("ParameterCurves").is_some() {
        return Some("CubismFadeMotionData".to_string());
    }
    if value.get("Parameters").is_some() && value.get("FadeInTime").is_some() && value.get("FadeOutTime").is_some() {
        if value.get("ParameterIds").is_none() {
            return Some("CubismExpressionData".to_string());
        }
    }
    if value.get("GroupIndex").is_some() && value.get("Link").is_some() && value.get("m_GameObject").is_some() {
        return Some("CubismPosePart".to_string());
    }
    None
}
struct Keyframe {
    time: f32,
    value: f32,
    in_slope: f32,
    out_slope: f32,
}
fn parse_float(val: Option<&UnityValue>) -> Option<f32> {
    match val {
        Some(UnityValue::Float(f)) => Some(*f),
        Some(UnityValue::Double(d)) => Some(*d as f32),
        Some(UnityValue::Int8(i)) => Some(*i as f32),
        Some(UnityValue::UInt8(u)) => Some(*u as f32),
        Some(UnityValue::Int16(i)) => Some(*i as f32),
        Some(UnityValue::UInt16(u)) => Some(*u as f32),
        Some(UnityValue::Int32(i)) => Some(*i as f32),
        Some(UnityValue::UInt32(u)) => Some(*u as f32),
        Some(UnityValue::Int64(i)) => Some(*i as f32),
        Some(UnityValue::UInt64(u)) => Some(*u as f32),
        _ => None,
    }
}
fn parse_keyframe(val: &UnityValue) -> Option<Keyframe> {
    let map = match val {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let time = parse_float(map.get("time"))?;
    let value = parse_float(map.get("value"))?;
    let in_slope = parse_float(map.get("inSlope"))
        .or_else(|| parse_float(map.get("in_slope")))
        .unwrap_or(0.0);
    let out_slope = parse_float(map.get("outSlope"))
        .or_else(|| parse_float(map.get("out_slope")))
        .unwrap_or(0.0);
    Some(Keyframe { time, value, in_slope, out_slope })
}
fn add_segments(
    curve: &Keyframe,
    pre_curve: &Keyframe,
    next_curve: Option<&Keyframe>,
    segments: &mut Vec<f32>,
    _force_bezier: bool,
    total_point_count: &mut i32,
    total_segment_count: &mut i32,
    j: &mut usize,
) {
    if (curve.time - pre_curve.time - 0.01).abs() < 0.0001 {
        if let Some(next) = next_curve {
            if next.value == curve.value {
                segments.push(3.0);
                segments.push(next.time);
                segments.push(next.value);
                *j += 1;
                *total_point_count += 1;
                *total_segment_count += 1;
                return;
            }
        }
    }
    if curve.in_slope.is_infinite() && curve.in_slope.is_sign_positive() || curve.in_slope > 1000000.0 {
        segments.push(2.0);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 1;
    } else if pre_curve.out_slope == 0.0 && curve.in_slope.abs() < 0.0001 {
        segments.push(0.0);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 1;
    } else {
        let tangent_length = (curve.time - pre_curve.time) / 3.0;
        segments.push(1.0);
        segments.push(pre_curve.time + tangent_length);
        segments.push(pre_curve.out_slope * tangent_length + pre_curve.value);
        segments.push(curve.time - tangent_length);
        segments.push(curve.value - curve.in_slope * tangent_length);
        segments.push(curve.time);
        segments.push(curve.value);
        *total_point_count += 3;
    }
    *total_segment_count += 1;
}
fn convert_fade_motion_to_json(value: &UnityValue) -> Option<serde_json::Value> {
    let map = match value {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let motion_length = parse_float(map.get("MotionLength")).unwrap_or(0.0);
    let fade_in_time = parse_float(map.get("FadeInTime")).unwrap_or(0.0);
    let fade_out_time = parse_float(map.get("FadeOutTime")).unwrap_or(0.0);
    let parameter_ids = match map.get("ParameterIds") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_fade_in_times = match map.get("ParameterFadeInTimes") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| parse_float(Some(v)).unwrap_or(-1.0)).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_fade_out_times = match map.get("ParameterFadeOutTimes") {
        Some(UnityValue::Array(arr)) => {
            arr.iter().map(|v| parse_float(Some(v)).unwrap_or(-1.0)).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let parameter_curves = match map.get("ParameterCurves") {
        Some(UnityValue::Array(arr)) => arr,
        _ => return None,
    };
    let mut curves_json = Vec::new();
    let mut total_segment_count = 0;
    let mut total_point_count = 0;
    for i in 0..parameter_curves.len() {
        let curve_val = &parameter_curves[i];
        let curve_map = match curve_val {
            UnityValue::Map(m) => m,
            _ => continue,
        };
        let m_curve = match curve_map.get("m_Curve") {
            Some(UnityValue::Array(arr)) => arr,
            _ => continue,
        };
        if m_curve.is_empty() {
            continue;
        }
        let keyframes: Vec<Keyframe> = m_curve.iter().filter_map(parse_keyframe).collect();
        if keyframes.is_empty() {
            continue;
        }
        let param_id = parameter_ids.get(i).cloned().unwrap_or_else(|| "".to_string());
        if param_id.is_empty() {
            continue;
        }
        let target = match param_id.as_str() {
            "Opacity" | "EyeBlink" | "LipSync" => "Model",
            _ => {
                if param_id.to_lowercase().contains("part") {
                    "PartOpacity"
                } else {
                    "Parameter"
                }
            }
        };
        let curve_fade_in = parameter_fade_in_times.get(i).cloned().unwrap_or(-1.0);
        let curve_fade_out = parameter_fade_out_times.get(i).cloned().unwrap_or(-1.0);
        let mut segments = vec![keyframes[0].time, keyframes[0].value];
        let mut j = 1;
        while j < keyframes.len() {
            let curve = &keyframes[j];
            let pre_curve = &keyframes[j - 1];
            let next_curve = keyframes.get(j + 1);
            add_segments(
                curve,
                pre_curve,
                next_curve,
                &mut segments,
                false,
                &mut total_point_count,
                &mut total_segment_count,
                &mut j,
            );
            j += 1;
        }
        total_point_count += 1;
        curves_json.push(serde_json::json!({
            "Target": target,
            "Id": param_id,
            "FadeInTime": curve_fade_in,
            "FadeOutTime": curve_fade_out,
            "Segments": segments,
        }));
    }
    let curve_count = curves_json.len();
    let motion_json = serde_json::json!({
        "Version": 3,
        "Meta": {
            "Duration": motion_length,
            "Fps": 30.0,
            "Loop": true,
            "AreBeziersRestricted": true,
            "FadeInTime": fade_in_time,
            "FadeOutTime": fade_out_time,
            "CurveCount": curve_count as i32,
            "TotalSegmentCount": total_segment_count,
            "TotalPointCount": total_point_count,
            "UserDataCount": 0,
            "TotalUserDataSize": 0
        },
        "Curves": curves_json,
        "UserData": []
    });
    Some(motion_json)
}
fn convert_expression_data_to_json(value: &UnityValue) -> Option<serde_json::Value> {
    let map = match value {
        UnityValue::Map(m) => m,
        _ => return None,
    };
    let exp_type = match map.get("Type") {
        Some(UnityValue::String(s)) => s.clone(),
        _ => "Live2D Expression".to_string(),
    };
    let fade_in_time = parse_float(map.get("FadeInTime")).unwrap_or(1.0);
    let fade_out_time = parse_float(map.get("FadeOutTime")).unwrap_or(1.0);
    let parameters = match map.get("Parameters") {
        Some(UnityValue::Array(arr)) => {
            let mut params_json = Vec::new();
            for item in arr {
                if let UnityValue::Map(item_map) = item {
                    let id = match item_map.get("Id") {
                        Some(UnityValue::String(s)) => s.clone(),
                        _ => continue,
                    };
                    let val = parse_float(item_map.get("Value")).unwrap_or(0.0);
                    let blend = match item_map.get("Blend") {
                        Some(v) => v.as_i32().unwrap_or(0),
                        _ => 0,
                    };
                    params_json.push(serde_json::json!({
                        "Id": id,
                        "Value": val,
                        "Blend": blend
                    }));
                }
            }
            params_json
        }
        _ => Vec::new(),
    };
    Some(serde_json::json!({
        "Type": exp_type,
        "FadeInTime": fade_in_time,
        "FadeOutTime": fade_out_time,
        "Parameters": parameters
    }))
}
fn flatten_json_paths(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.contains('/') || s.contains('\\') {
                if let Some(filename) = Path::new(s).file_name() {
                    *s = filename.to_string_lossy().into_owned();
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                flatten_json_paths(v);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                flatten_json_paths(v);
            }
        }
        _ => {}
    }
}
fn extract_monobehaviour(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    asset_name: &str,
    obj: &unityfs::objectreader::ObjectReader,
    by_file: bool,
    pb: &indicatif::ProgressBar,
    pose_parts: &std::sync::Mutex<Vec<PosePartData>>,
    moc_stem: &std::sync::Mutex<Option<String>>,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "".to_string(),
    };
    let base_name = if !name.is_empty() {
        Path::new(&name)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or(name)
    } else {
        format!("monobehaviour_{}", obj.path_id)
    };
    let sanitized_base = base_name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
    if sanitized_base.is_empty() {
        return false;
    }
    if find_and_extract_criware_bytes(val, output_dir, &sanitized_base, "", by_file, pb) {
        return true;
    }
    let mut is_cubism_pose_part = false;
    if let Some(class_name) = resolve_class_name(val, asset_manager, asset_name) {
        if class_name == "CubismPosePart" {
            is_cubism_pose_part = true;
        }
    }
    if is_cubism_pose_part {
        let group_index = val.get("GroupIndex").and_then(|v| v.as_i32()).unwrap_or(0);
        let link = val.get("Link").and_then(|v| match v {
            UnityValue::Array(arr) => Some(arr.iter().map(|item| item.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()),
            _ => None
        }).unwrap_or_default();
        let mut go_name = String::new();
        if let Some(go_pptr) = val.get("m_GameObject") {
            if let UnityValue::PPtr { file_id, path_id } = go_pptr {
                if let Ok(go_val) = asset_manager.read_object_value(asset_name, *file_id, *path_id) {
                    if let Some(UnityValue::String(name)) = go_val.get("m_Name") {
                        go_name = name.clone();
                    }
                }
            }
        }
        if !go_name.is_empty() {
            let mut guard = pose_parts.lock().unwrap();
            guard.push(PosePartData {
                id: go_name,
                group_index,
                link,
            });
        }
        return true;
    }
    let mut content = None;
    let mut is_cubism_fade_motion = false;
    let mut is_cubism_expression = false;
    if let Some(class_name) = resolve_class_name(val, asset_manager, asset_name) {
        if class_name == "CubismFadeMotionData" {
            is_cubism_fade_motion = true;
        } else if class_name == "CubismExpressionData" {
            is_cubism_expression = true;
        }
    }
    if is_cubism_fade_motion {
        if let Some(json_val) = convert_fade_motion_to_json(val) {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                content = Some(bytes);
            }
        }
    } else if is_cubism_expression {
        if let Some(json_val) = convert_expression_data_to_json(val) {
            if let Ok(bytes) = serde_json::to_vec_pretty(&json_val) {
                content = Some(bytes);
            }
        }
    }
    if content.is_none() {
        if let Some(sf) = asset_manager.files.get(asset_name) {
            let start = obj.byte_start;
            let size = obj.byte_size;
            if start + size <= sf.data.len() {
                let raw_bytes = &sf.data[start .. start + size];
                if let Some(moc_pos) = raw_bytes.windows(4).position(|window| window == b"MOC3") {
                    if moc_pos >= 4 {
                        let size_bytes = &raw_bytes[moc_pos - 4 .. moc_pos];
                        let is_big_endian = match sf.endian {
                            unityfs::reader::Endian::Big => true,
                            unityfs::reader::Endian::Little => false,
                        };
                        let moc_size = if is_big_endian {
                            u32::from_be_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                        } else {
                            u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]) as usize
                        };
                        if moc_pos + moc_size <= raw_bytes.len() {
                            content = Some(raw_bytes[moc_pos .. moc_pos + moc_size].to_vec());
                        } else {
                            content = Some(raw_bytes[moc_pos..].to_vec());
                        }
                    } else {
                        content = Some(raw_bytes[moc_pos..].to_vec());
                    }
                }
            }
        }
    }
    if content.is_none() {
        for key in &["_bytes", "m_Bytes", "bytes", "m_Data", "_data"] {
            if let Some(UnityValue::Bytes(b)) = val.get(*key) {
                content = Some(b.clone());
                break;
            }
        }
    }
    if content.is_none() {
        if let UnityValue::Map(map) = val {
            for (_, val_item) in map {
                if let UnityValue::Bytes(b) = val_item {
                    if b.starts_with(b"MOC3") {
                        content = Some(b.clone());
                        break;
                    }
                }
            }
        }
    }
    if let Some(data) = content {
        let mut final_name = sanitized_base.clone();
        if is_cubism_expression || final_name.ends_with(".exp") || final_name.ends_with(".exp3") {
            let name_without_exp = final_name.replace(".exp3", "").replace(".exp", "");
            final_name = format!("{}.exp3.json", name_without_exp);
        } else if is_cubism_fade_motion || final_name.ends_with(".fade") || final_name.ends_with(".motion") || final_name.ends_with(".motion3") {
            let name_without_motion = final_name
                .replace(".motion3", "")
                .replace(".motion", "")
                .replace(".fade", "");
            final_name = format!("{}.motion3.json", name_without_motion);
        } else if final_name.ends_with(".model") || final_name.ends_with(".model3") {
            let name_without = final_name.replace(".model3", "").replace(".model", "");
            final_name = format!("{}.model3.json", name_without);
        } else if final_name.ends_with(".physics") || final_name.ends_with(".physics3") {
            let name_without = final_name.replace(".physics3", "").replace(".physics", "");
            final_name = format!("{}.physics3.json", name_without);
        } else if final_name.ends_with(".pose") || final_name.ends_with(".pose3") {
            let name_without = final_name.replace(".pose3", "").replace(".pose", "");
            final_name = format!("{}.pose3.json", name_without);
        } else if final_name.ends_with(".cdi") || final_name.ends_with(".cdi3") {
            let name_without = final_name.replace(".cdi3", "").replace(".cdi", "");
            final_name = format!("{}.cdi3.json", name_without);
        } else if final_name.ends_with(".userdata") || final_name.ends_with(".userdata3") {
            let name_without = final_name.replace(".userdata3", "").replace(".userdata", "");
            final_name = format!("{}.userdata3.json", name_without);
        } else if !final_name.contains('.') {
            if data.starts_with(b"{") {
                final_name = format!("{}.json", final_name);
            } else if data.starts_with(b"MOC3") {
                final_name = format!("{}.moc3", final_name);
            } else {
                let check_len = std::cmp::min(data.len(), 256);
                let head_str = String::from_utf8_lossy(&data[..check_len]);
                let has_spine_version = head_str.contains("3.6") || head_str.contains("3.7") ||
                                        head_str.contains("3.8") || head_str.contains("4.0") ||
                                        head_str.contains("4.1") || head_str.contains("4.2");
                if has_spine_version {
                    final_name = format!("{}.skel", final_name);
                } else {
                    final_name = format!("{}.txt", final_name);
                }
            }
        }
        let monobehaviour_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("MonoBehaviour")
        };
        let _ = std::fs::create_dir_all(&monobehaviour_dir);
        let mut final_data = data;
        if final_name.ends_with(".model3.json") {
            if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&final_data) {
                flatten_json_paths(&mut json_val);
                if let Ok(serialized) = serde_json::to_vec_pretty(&json_val) {
                    final_data = serialized;
                }
            }
        }
        if final_name.ends_with(".moc3") {
            let mut guard = moc_stem.lock().unwrap();
            *guard = Some(sanitized_base.replace(".moc3", "").replace(".moc", ""));
        }
        match get_unique_path(&monobehaviour_dir, &final_name, Some(&final_data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &final_data) {
                    pb.println(format!("    Failed to write MonoBehaviour asset '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}
fn extract_bundle_file(file_path: &Path, base_output_dir: &Path, filter: &ExtractFilter, pb: &indicatif::ProgressBar) {
    let file_stem = file_path
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("bundle"))
        .to_string_lossy();
    let file = match File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            pb.println(format!("[{}] Failed to open file '{}': {}", file_stem, file_path.display(), e));
            return;
        }
    };
    let mmap = match unsafe { memmap2::Mmap::map(&file) } {
        Ok(m) => m,
        Err(e) => {
            pb.println(format!("[{}] Failed to memory map file '{}': {}", file_stem, file_path.display(), e));
            return;
        }
    };
    let mut reader = unityfs::Reader::new_mmap(mmap, unityfs::UnityVersion::default());
    let bundle = match unityfs::Bundle::read(&mut reader) {
        Ok(b) => b,
        Err(e) => {
            pb.println(format!("[{}] Failed to read bundle: {}", file_stem, e));
            return;
        }
    };
    let bundle_output_dir = if filter.live2d {
        base_output_dir.join(get_model_base_name(&file_stem))
    } else if filter.by_file {
        base_output_dir.join(file_stem.as_ref())
    } else {
        base_output_dir.to_path_buf()
    };
    if let Err(e) = std::fs::create_dir_all(&bundle_output_dir) {
        pb.println(format!("[{}] Failed to create output directory '{}': {}", file_stem, bundle_output_dir.display(), e));
        return;
    }
    let mut asset_manager = AssetManager::new();
    for entry in &bundle.files {
        if entry.name.ends_with(".resS") || entry.name.ends_with(".resource") {
            asset_manager.add_raw_file(entry.name.clone(), entry.data.clone());
        } else if entry.data.len() > 20 {
            let mut sf_reader = unityfs::Reader::new(entry.data.clone(), bundle.engine_version.clone());
            let sf = unityfs::SerializedFile::read(&mut sf_reader);
            asset_manager.add_file(entry.name.clone(), sf);
        }
    }
    let mut all_objects = Vec::new();
    for (asset_name, sf) in &asset_manager.files {
        for obj in &sf.objects {
            all_objects.push((asset_name, obj));
        }
    }
    let objects_to_extract: Vec<_> = all_objects
        .into_par_iter()
        .filter(|(asset_name, obj)| {
            let class_id = obj.class_id;
            let is_supported = match class_id {
                28 | 49 | 43 | 83 | 48 | 329 | 114 => true,
                1 | 4 | 21 | 74 | 115 if filter.extract_metadata => true,
                _ => false,
            };
            if !is_supported {
                return false;
            }
            let t_name = obj.type_name();
            if let Some(ref filter_t) = filter.filter_type {
                if !t_name.to_lowercase().contains(&filter_t.to_lowercase()) {
                    return false;
                }
            }
            match asset_manager.read_object_value(asset_name, 0, obj.path_id) {
                Ok(unity_value) => {
                    let m_name = match unity_value.get("m_Name") {
                        Some(UnityValue::String(s)) => s.clone(),
                        _ => "".to_string(),
                    };
                    if let Some(ref filter_n) = filter.filter_name {
                        if !m_name.to_lowercase().contains(&filter_n.to_lowercase()) {
                            return false;
                        }
                    }
                    true
                }
                Err(e) => {
                    pb.println(format!("[{}]     Warning: Failed to read object value for path_id {}: {}", file_stem, obj.path_id, e));
                    false
                }
            }
        })
        .collect();
    if objects_to_extract.is_empty() {
        return;
    }
    let pose_parts = std::sync::Mutex::new(Vec::new());
    let moc_stem = std::sync::Mutex::new(None);
    let _: Vec<String> = objects_to_extract
        .par_iter()
        .filter_map(|(asset_name, obj)| {
            let class_id = obj.class_id;
            let path_id = obj.path_id;
            let t_name = obj.type_name();
            let mut res = None;
            if let Ok(unity_value) = asset_manager.read_object_value(asset_name, 0, path_id) {
                let m_name = match unity_value.get("m_Name") {
                    Some(UnityValue::String(s)) => s.clone(),
                    _ => "".to_string(),
                };
                pb.set_message(format!("{}: {}", t_name, m_name));
                let success = match class_id {
                    28 => {
                        extract_texture2d(&unity_value, &bundle_output_dir, &asset_manager, filter.by_file || filter.live2d, pb)
                    }
                    49 => {
                        extract_text_asset(&unity_value, &bundle_output_dir, filter.by_file || filter.live2d, pb)
                    }
                    43 => {
                        extract_mesh(&unity_value, &bundle_output_dir, filter.by_file || filter.live2d, pb)
                    }
                    83 => {
                        extract_audioclip(&unity_value, &bundle_output_dir, &asset_manager, filter.by_file || filter.live2d, pb)
                    }
                    48 => {
                        extract_shader(&unity_value, &bundle_output_dir, filter.by_file || filter.live2d, pb)
                    }
                    329 => {
                        extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, filter.by_file || filter.live2d, pb)
                    }
                    114 => {
                        let success = extract_monobehaviour(
                            &unity_value,
                            &bundle_output_dir,
                            &asset_manager,
                            asset_name,
                            obj,
                            filter.by_file || filter.live2d,
                            pb,
                            &pose_parts,
                            &moc_stem,
                        );
                        if filter.extract_metadata {
                            dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, filter.by_file || filter.live2d, path_id);
                        }
                        success
                    }
                    1 | 4 | 21 | 74 | 115 => {
                        if filter.extract_metadata {
                            dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, filter.by_file || filter.live2d, path_id)
                        } else {
                            false
                        }
                    }
                    _ => false
                };
                if success {
                    res = Some(t_name.to_string());
                }
            }
            res
        })
        .collect();
    let pose_parts_vec = pose_parts.into_inner().unwrap();
    if !pose_parts_vec.is_empty() {
        let stem = moc_stem.into_inner().unwrap().unwrap_or_else(|| {
            file_stem.to_string()
        });
        let monobehaviour_dir = if filter.by_file || filter.live2d {
            bundle_output_dir.to_path_buf()
        } else {
            bundle_output_dir.join("MonoBehaviour")
        };
        let mut group_map: std::collections::BTreeMap<i32, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
        for part in pose_parts_vec {
            let node = serde_json::json!({
                "Id": part.id,
                "Link": part.link,
            });
            group_map.entry(part.group_index).or_default().push(node);
        }
        let groups: Vec<Vec<serde_json::Value>> = group_map.into_values().collect();
        let pose_json = serde_json::json!({
            "Type": "Live2D Pose",
            "Groups": groups,
        });
        if let Ok(serialized) = serde_json::to_vec_pretty(&pose_json) {
            let _ = std::fs::create_dir_all(&monobehaviour_dir);
            match get_unique_path(&monobehaviour_dir, &format!("{}.pose3.json", stem), Some(&serialized)) {
                UniquePathResult::New(dest) => {
                    if let Err(e) = std::fs::write(&dest, &serialized) {
                        pb.println(format!("    Failed to write pose asset '{}': {}", dest.display(), e));
                    }
                }
                UniquePathResult::Exists(_) => {}
            }
        }
    }
}
fn is_json_bytes(data: &[u8]) -> bool {
    if let Ok(s) = std::str::from_utf8(data) {
        let trimmed = s.trim_start();
        trimmed.starts_with('{') || trimmed.starts_with('[')
    } else {
        false
    }
}
fn extract_text_asset(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &indicatif::ProgressBar) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "text_asset".to_string(),
    };
    let content = match val.get("m_Script") {
        Some(UnityValue::String(s)) => Some(s.as_bytes().to_vec()),
        Some(UnityValue::Bytes(b)) => Some(b.clone()),
        _ => None,
    };
    if let Some(data) = content {
        let is_moc3 = data.starts_with(b"MOC3");
        let name_lower = name.to_lowercase();
        let filename = if is_moc3 {
            let mut stem = name.clone();
            if stem.to_lowercase().ends_with(".moc3") {
                stem = stem[..stem.len() - 5].to_string();
            } else if stem.to_lowercase().ends_with(".moc") {
                stem = stem[..stem.len() - 4].to_string();
            } else if stem.to_lowercase().ends_with("_moc3") {
                stem = stem[..stem.len() - 5].to_string();
            } else if stem.to_lowercase().ends_with("_moc") {
                stem = stem[..stem.len() - 4].to_string();
            }
            format!("{}.moc3", stem)
        } else if is_json_bytes(&data) {
            let mut base = name.clone();
            if base.to_lowercase().ends_with(".json") {
                base = base[..base.len() - 5].to_string();
            }
            if name_lower.contains("physics") {
                if base.to_lowercase().ends_with(".physics3") || base.to_lowercase().ends_with("_physics3") {
                    let stem = base[..base.len() - 9].to_string();
                    format!("{}.physics3.json", stem)
                } else if base.to_lowercase().ends_with(".physics") || base.to_lowercase().ends_with("_physics") {
                    let stem = base[..base.len() - 8].to_string();
                    format!("{}.physics.json", stem)
                } else {
                    format!("{}.physics3.json", base)
                }
            } else if name_lower.contains("motion") {
                if base.to_lowercase().ends_with(".motion3") || base.to_lowercase().ends_with("_motion3") {
                    let stem = base[..base.len() - 8].to_string();
                    format!("{}.motion3.json", stem)
                } else if base.to_lowercase().ends_with(".motion") || base.to_lowercase().ends_with("_motion") {
                    let stem = base[..base.len() - 7].to_string();
                    format!("{}.motion.json", stem)
                } else {
                    format!("{}.motion3.json", base)
                }
            } else if name_lower.contains("pose") {
                if base.to_lowercase().ends_with(".pose3") || base.to_lowercase().ends_with("_pose3") {
                    let stem = base[..base.len() - 6].to_string();
                    format!("{}.pose3.json", stem)
                } else if base.to_lowercase().ends_with(".pose") || base.to_lowercase().ends_with("_pose") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.pose.json", stem)
                } else {
                    format!("{}.pose.json", base)
                }
            } else if name_lower.contains("cdi") {
                if base.to_lowercase().ends_with(".cdi3") || base.to_lowercase().ends_with("_cdi3") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.cdi3.json", stem)
                } else {
                    format!("{}.cdi3.json", base)
                }
            } else if name_lower.contains("userdata") {
                if base.to_lowercase().ends_with(".userdata3") || base.to_lowercase().ends_with("_userdata3") {
                    let stem = base[..base.len() - 10].to_string();
                    format!("{}.userdata3.json", stem)
                } else {
                    format!("{}.userdata3.json", base)
                }
            } else if name_lower.contains("exp") {
                if base.to_lowercase().ends_with(".exp3") || base.to_lowercase().ends_with("_exp3") {
                    let stem = base[..base.len() - 5].to_string();
                    format!("{}.exp3.json", stem)
                } else if base.to_lowercase().ends_with(".exp") || base.to_lowercase().ends_with("_exp") {
                    let stem = base[..base.len() - 4].to_string();
                    format!("{}.exp.json", stem)
                } else {
                    format!("{}.exp3.json", base)
                }
            } else {
                format!("{}.json", base)
            }
        } else {
            let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
            if safe_name.is_empty() {
                "text_asset.txt".to_string()
            } else if safe_name.contains('.') {
                safe_name
            } else {
                let check_len = std::cmp::min(data.len(), 256);
                let head_str = String::from_utf8_lossy(&data[..check_len]);
                let has_spine_version = head_str.contains("3.6") || head_str.contains("3.7") ||
                                        head_str.contains("3.8") || head_str.contains("4.0") ||
                                        head_str.contains("4.1") || head_str.contains("4.2");
                if has_spine_version {
                    format!("{}.skel", safe_name)
                } else {
                    format!("{}.txt", safe_name)
                }
            }
        };
        let text_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("TextAsset")
        };
        let _ = std::fs::create_dir_all(&text_dir);
        match get_unique_path(&text_dir, &filename, Some(&data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write text asset '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}
fn extract_texture2d(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &indicatif::ProgressBar,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "texture".to_string(),
    };
    let width = match val.get("m_Width") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
        _ => 0,
    };
    let height = match val.get("m_Height") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0) as usize,
        _ => 0,
    };
    let format = match val.get("m_TextureFormat") {
        Some(v) => i32::try_from_unity_value(v).unwrap_or(0),
        _ => 0,
    };
    if width == 0 || height == 0 {
        return false;
    }
    let mut image_data = Vec::new();
    let mut has_stream = false;
    if let Some(UnityValue::Map(stream_map)) = val.get("m_StreamData") {
        let offset = stream_map.get("offset").and_then(|v| match v {
            UnityValue::UInt64(o) => Some(*o),
            UnityValue::Int64(o) => Some(*o as u64),
            UnityValue::UInt32(o) => Some(*o as u64),
            UnityValue::Int32(o) => Some(*o as u64),
            _ => None,
        });
        let size = stream_map.get("size").and_then(|v| match v {
            UnityValue::UInt64(s) => Some(*s as u32),
            UnityValue::Int64(s) => Some(*s as u32),
            UnityValue::UInt32(s) => Some(*s),
            UnityValue::Int32(s) => Some(*s as u32),
            _ => None,
        });
        let path = stream_map.get("path").and_then(|v| match v {
            UnityValue::String(s) => Some(s.clone()),
            _ => None,
        });
        if let (Some(o), Some(s), Some(p)) = (offset, size, path) {
            if s > 0 {
                let stream_name = p.rsplit('/').next().unwrap_or(&p);
                if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                    let start = o as usize;
                    let end = start + s as usize;
                    if end <= raw_data.len() {
                        image_data = raw_data[start..end].to_vec();
                        has_stream = true;
                    }
                }
            }
        }
    }
    if !has_stream {
        if let Some(UnityValue::Bytes(b)) = val.get("image_data").or_else(|| val.get("image data")) {
            image_data = b.clone();
        }
    }
    if image_data.is_empty() {
        return false;
    }
    if let Some(rgba_data) = decompress_texture(width, height, format, &image_data) {
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "texture.png".to_string()
        } else {
            format!("{}.png", safe_name)
        };
        let texture_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Texture2D")
        };
        let _ = std::fs::create_dir_all(&texture_dir);
        let dest = match get_unique_path(&texture_dir, &filename, None) {
            UniquePathResult::New(dest) | UniquePathResult::Exists(dest) => dest,
        };
        if let Err(e) = image::save_buffer(
            &dest,
            &rgba_data,
            width as u32,
            height as u32,
            image::ExtendedColorType::Rgba8,
        ) {
            pb.println(format!("    Failed to save PNG texture '{}': {}", dest.display(), e));
            false
        } else {
            true
        }
    } else {
        pb.println(format!("    Failed to decompress texture '{}' (format={})", name, format));
        false
    }
}
fn extract_mesh(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &indicatif::ProgressBar) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "mesh".to_string(),
    };
    if let Ok(mesh) = Mesh::try_from_unity_value(val) {
        let vertices = mesh.get_vertices().unwrap_or_default();
        let indices = mesh.get_indices().unwrap_or_default();
        let uvs = mesh.extract_uvs().unwrap_or_default();
        let normals = mesh.get_normals().unwrap_or_default();
        if vertices.is_empty() {
            return false;
        }
        let mut obj_content = String::new();
        obj_content.push_str(&format!("# Mesh Exporter: {}\n", name));
        obj_content.push_str(&format!("o {}\n\n", name));
        for v in &vertices {
            obj_content.push_str(&format!("v {} {} {}\n", v.x, v.y, v.z));
        }
        obj_content.push_str("\n");
        for uv in &uvs {
            obj_content.push_str(&format!("vt {} {}\n", uv.0, uv.1));
        }
        obj_content.push_str("\n");
        for n in &normals {
            obj_content.push_str(&format!("vn {} {} {}\n", n.x, n.y, n.z));
        }
        obj_content.push_str("\n");
        let has_uv = !uvs.is_empty();
        let has_normal = !normals.is_empty();
        for chunk in indices.chunks_exact(3) {
            let i1 = chunk[0] + 1;
            let i2 = chunk[1] + 1;
            let i3 = chunk[2] + 1;
            match (has_uv, has_normal) {
                (true, true) => {
                    obj_content.push_str(&format!("f {}/{}/{} {}/{}/{} {}/{}/{}\n", i1, i1, i1, i2, i2, i2, i3, i3, i3));
                }
                (true, false) => {
                    obj_content.push_str(&format!("f {}/{} {}/{} {}/{}\n", i1, i1, i2, i2, i3, i3));
                }
                (false, true) => {
                    obj_content.push_str(&format!("f {}//{} {}//{} {}//{}\n", i1, i1, i2, i2, i3, i3));
                }
                (false, false) => {
                    obj_content.push_str(&format!("f {} {} {}\n", i1, i2, i3));
                }
            }
        }
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "mesh.obj".to_string()
        } else {
            format!("{}.obj", safe_name)
        };
        let mesh_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Mesh")
        };
        let _ = std::fs::create_dir_all(&mesh_dir);
        match get_unique_path(&mesh_dir, &filename, Some(obj_content.as_bytes())) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, obj_content) {
                    pb.println(format!("    Failed to write Mesh OBJ '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}
fn extract_audioclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &indicatif::ProgressBar,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "audio".to_string(),
    };
    let mut audio_data = Vec::new();
    let mut has_stream = false;
    if let Some(UnityValue::Map(res_map)) = val.get("m_Resource") {
        let offset = res_map.get("m_Offset").and_then(|v| match v {
            UnityValue::UInt64(o) => Some(*o),
            UnityValue::Int64(o) => Some(*o as u64),
            UnityValue::UInt32(o) => Some(*o as u64),
            UnityValue::Int32(o) => Some(*o as u64),
            _ => None,
        });
        let size = res_map.get("m_Size").and_then(|v| match v {
            UnityValue::UInt64(s) => Some(*s as u32),
            UnityValue::Int64(s) => Some(*s as u32),
            UnityValue::UInt32(s) => Some(*s),
            UnityValue::Int32(s) => Some(*s as u32),
            _ => None,
        });
        let path = res_map.get("m_Source").and_then(|v| match v {
            UnityValue::String(s) => Some(s.clone()),
            _ => None,
        });
        if let (Some(o), Some(s), Some(p)) = (offset, size, path) {
            if s > 0 {
                let stream_name = p.rsplit('/').next().unwrap_or(&p);
                if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                    let start = o as usize;
                    let end = start + s as usize;
                    if end <= raw_data.len() {
                        audio_data = raw_data[start..end].to_vec();
                        has_stream = true;
                    }
                }
            }
        }
    }
    if !has_stream {
        if let Some(UnityValue::Bytes(b)) = val.get("m_AudioData") {
            audio_data = b.clone();
        }
    }
    if audio_data.is_empty() {
        return false;
    }
    let extension = if audio_data.starts_with(b"OggS") {
        "ogg"
    } else if audio_data.starts_with(b"RIFF") {
        "wav"
    } else if audio_data.len() >= 8 && &audio_data[4..8] == b"ftyp" {
        "m4a"
    } else if audio_data.starts_with(b"ID3") || audio_data.starts_with(&[0xFF, 0xFB]) {
        "mp3"
    } else if audio_data.starts_with(b"MThd") {
        "mid"
    } else if audio_data.starts_with(b"FSB5") {
        "fsb"
    } else {
        "bytes"
    };
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if safe_name.is_empty() {
        format!("audio.{}", extension)
    } else {
        format!("{}.{}", safe_name, extension)
    };
    let audio_dir = if by_file {
        output_dir.to_path_buf()
    } else {
        output_dir.join("AudioClip")
    };
    let _ = std::fs::create_dir_all(&audio_dir);
    match get_unique_path(&audio_dir, &filename, Some(&audio_data)) {
        UniquePathResult::New(dest) => {
            if let Err(e) = std::fs::write(&dest, &audio_data) {
                pb.println(format!("    Failed to write AudioClip '{}': {}", dest.display(), e));
                false
            } else {
                true
            }
        }
        UniquePathResult::Exists(_) => true,
    }
}
fn extract_videoclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
    by_file: bool,
    pb: &indicatif::ProgressBar,
) -> bool {
    let video_clip = match unityfs::classes::videoclip::VideoClip::from_value(val.clone()) {
        Ok(vc) => vc,
        Err(e) => {
            pb.println(format!("    Failed to parse VideoClip structure: {}", e));
            return false;
        }
    };
    if video_clip.m_ExternalResources.m_Size > 0 {
        let mut video_data = Vec::new();
        let source = &video_clip.m_ExternalResources.m_Source;
        if !source.is_empty() {
            let stream_name = source.rsplit('/').next().unwrap_or(source);
            if let Some(raw_data) = asset_manager.raw_files.get(stream_name) {
                let start = video_clip.m_ExternalResources.m_Offset as usize;
                let end = start + video_clip.m_ExternalResources.m_Size as usize;
                if end <= raw_data.len() {
                    video_data = raw_data[start..end].to_vec();
                }
            }
        }
        if !video_data.is_empty() {
            let ext = video_clip.m_OriginalPath.as_ref()
                .and_then(|p| Path::new(p).extension())
                .and_then(|s| s.to_str())
                .unwrap_or("mp4");
            let dot_ext = format!(".{}", ext);
            let sanitized_base = video_clip.m_Name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
            let filename = if sanitized_base.is_empty() {
                format!("video.{}", ext)
            } else if sanitized_base.ends_with(&dot_ext) {
                sanitized_base.clone()
            } else {
                format!("{}{}", sanitized_base, dot_ext)
            };
            let video_dir = if by_file {
                output_dir.to_path_buf()
            } else {
                output_dir.join("VideoClip")
            };
            let _ = std::fs::create_dir_all(&video_dir);
            match get_unique_path(&video_dir, &filename, Some(&video_data)) {
                UniquePathResult::New(dest) => {
                    if let Err(e) = std::fs::write(&dest, &video_data) {
                        pb.println(format!("    Failed to write VideoClip '{}': {}", dest.display(), e));
                        false
                    } else {
                        true
                    }
                }
                UniquePathResult::Exists(_) => true,
            }
        } else {
            pb.println(format!("    Failed to extract VideoClip raw bytes: resource data is missing or empty."));
            false
        }
    } else {
        false
    }
}
fn extract_shader(val: &UnityValue, output_dir: &Path, by_file: bool, pb: &indicatif::ProgressBar) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => "shader".to_string(),
    };
    let content = match val.get("m_Script") {
        Some(UnityValue::String(s)) => Some(s.as_bytes().to_vec()),
        Some(UnityValue::Bytes(b)) => Some(b.clone()),
        _ => None,
    };
    if let Some(data) = content {
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
        let filename = if safe_name.is_empty() {
            "shader.shader".to_string()
        } else {
            format!("{}.shader", safe_name)
        };
        let shader_dir = if by_file {
            output_dir.to_path_buf()
        } else {
            output_dir.join("Shader")
        };
        let _ = std::fs::create_dir_all(&shader_dir);
        match get_unique_path(&shader_dir, &filename, Some(&data)) {
            UniquePathResult::New(dest) => {
                if let Err(e) = std::fs::write(&dest, &data) {
                    pb.println(format!("    Failed to write Shader '{}': {}", dest.display(), e));
                    false
                } else {
                    true
                }
            }
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}
fn dump_asset_as_json(
    class_id: i32,
    type_name: &str,
    val: &UnityValue,
    output_dir: &Path,
    by_file: bool,
    path_id: i64,
) -> bool {
    let name = match val.get("m_Name") {
        Some(UnityValue::String(s)) if !s.is_empty() => s.clone(),
        _ => type_name.to_lowercase(),
    };
    let json_val = unity_value_to_json(val);
    let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    let filename = if safe_name.is_empty() {
        format!("{}_{}_{}.json", type_name.to_lowercase(), class_id, path_id)
    } else {
        format!("{}_{}.json", safe_name, path_id)
    };
    let sub_dir_name = match class_id {
        21 => "materials",
        74 => "animationclips",
        114 => "monobehaviours",
        1 => "gameobjects",
        4 => "transforms",
        _ => "meta",
    };
    let target_dir = if by_file {
        output_dir.to_path_buf()
    } else {
        output_dir.join(sub_dir_name)
    };
    let _ = std::fs::create_dir_all(&target_dir);
    if let Ok(json_str) = serde_json::to_string_pretty(&json_val) {
        match get_unique_path(&target_dir, &filename, Some(json_str.as_bytes())) {
            UniquePathResult::New(dest) => std::fs::write(&dest, json_str).is_ok(),
            UniquePathResult::Exists(_) => true,
        }
    } else {
        false
    }
}
pub fn unity_value_to_json(value: &UnityValue) -> serde_json::Value {
    match value {
        UnityValue::Boolean(b) => serde_json::Value::Bool(*b),
        UnityValue::Int8(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt8(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int16(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt16(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int32(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt32(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Int64(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        UnityValue::UInt64(u) => serde_json::Value::Number(serde_json::Number::from(*u)),
        UnityValue::Float(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f as f64).unwrap_or_else(|| serde_json::Number::from(0))),
        UnityValue::Double(d) => serde_json::Value::Number(serde_json::Number::from_f64(*d).unwrap_or_else(|| serde_json::Number::from(0))),
        UnityValue::String(s) => serde_json::Value::String(s.clone()),
        UnityValue::Bytes(b) => {
            let arr = b.iter().map(|&x| serde_json::Value::Number(serde_json::Number::from(x))).collect();
            serde_json::Value::Array(arr)
        }
        UnityValue::Array(arr) => {
            let json_arr = arr.iter().map(unity_value_to_json).collect();
            serde_json::Value::Array(json_arr)
        }
        UnityValue::Map(map) => {
            let mut json_map = serde_json::Map::new();
            for (k, v) in map {
                let key = if k == "m_Name" { "name".to_string() } else { k.clone() };
                json_map.insert(key, unity_value_to_json(v));
            }
            serde_json::Value::Object(json_map)
        }
        UnityValue::PPtr { file_id, path_id } => {
            let mut json_map = serde_json::Map::new();
            json_map.insert("file_id".to_string(), serde_json::Value::Number(serde_json::Number::from(*file_id)));
            json_map.insert("path_id".to_string(), serde_json::Value::String(path_id.to_string()));
            json_map.insert("m_FileID".to_string(), serde_json::Value::Number(serde_json::Number::from(*file_id)));
            json_map.insert("m_PathID".to_string(), serde_json::Value::String(path_id.to_string()));
            serde_json::Value::Object(json_map)
        }
        UnityValue::Null => serde_json::Value::Null,
    }
}
fn decompress_texture(width: usize, height: usize, format: i32, image_data: &[u8]) -> Option<Vec<u8>> {
    let (block_w, block_h) = match format {
        10 | 11 | 12 | 24 | 25 | 26 | 27 | 34 | 35 | 36 | 41 | 42 | 43 | 44 | 45 | 46 | 47 => (4, 4),
        30 | 31 => (8, 4),
        32 | 33 => (4, 4),
        48 | 54 | 66 => (4, 4),
        49 | 55 | 67 => (5, 5),
        50 | 56 | 68 => (6, 6),
        51 | 57 | 69 => (8, 8),
        52 | 58 | 70 => (10, 10),
        53 | 59 | 71 => (12, 12),
        _ => (1, 1),
    };
    let aligned_w = ((width + block_w - 1) / block_w) * block_w;
    let aligned_h = ((height + block_h - 1) / block_h) * block_h;
    let aligned_size = aligned_w * aligned_h;
    let is_crunch = matches!(format, 28 | 29 | 64 | 65);
    let buffer_size = if is_crunch {
        width * height * 2
    } else {
        std::cmp::max(width * height, aligned_size)
    };
    let mut decompressed = vec![0u32; buffer_size];
    let mut success = false;
    let expected_input_size = match format {
        10 | 34 | 45 | 46 | 60 | 61 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 8)
        }
        12 | 25 | 27 | 47 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 16)
        }
        26 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 8)
        }
        48 | 54 | 66 => {
            Some(((width + 3) / 4) * ((height + 3) / 4) * 16)
        }
        49 | 55 | 67 => {
            Some(((width + 4) / 5) * ((height + 4) / 5) * 16)
        }
        50 | 56 | 68 => {
            Some(((width + 5) / 6) * ((height + 5) / 6) * 16)
        }
        51 | 57 | 69 => {
            Some(((width + 7) / 8) * ((height + 7) / 8) * 16)
        }
        52 | 58 | 70 => {
            Some(((width + 9) / 10) * ((height + 9) / 10) * 16)
        }
        53 | 59 | 71 => {
            Some(((width + 11) / 12) * ((height + 11) / 12) * 16)
        }
        _ => None,
    };
    let safe_image_data = match expected_input_size {
        Some(size) if image_data.len() >= size => &image_data[0..size],
        _ => image_data,
    };
    match format {
        28 | 29 | 64 | 65 => {
            success = texture2ddecoder::decode_unity_crunch(image_data, width, height, &mut decompressed).is_ok();
        }
        1 => {
            for (i, &a) in safe_image_data.iter().enumerate().take(width * height) {
                decompressed[i] = ((a as u32) << 24) | 0x00FFFFFF;
            }
            success = true;
        }
        2 => {
            for (i, chunk) in safe_image_data.chunks_exact(2).enumerate().take(width * height) {
                let val = u16::from_le_bytes([chunk[0], chunk[1]]);
                let a = ((val >> 12) & 0xF) as u8 * 17;
                let r = ((val >> 8) & 0xF) as u8 * 17;
                let g = ((val >> 4) & 0xF) as u8 * 17;
                let b = (val & 0xF) as u8 * 17;
                decompressed[i] = u32::from_le_bytes([b, g, r, a]);
            }
            success = true;
        }
        3 => {
            for (i, chunk) in safe_image_data.chunks_exact(3).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], 255]);
            }
            success = true;
        }
        4 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
            success = true;
        }
        5 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[3], chunk[2], chunk[1], chunk[0]]);
            }
            success = true;
        }
        7 => {
            for (i, chunk) in safe_image_data.chunks_exact(2).enumerate().take(width * height) {
                let val = u16::from_le_bytes([chunk[0], chunk[1]]);
                let r = ((val >> 11) & 0x1F) as u8;
                let g = ((val >> 5) & 0x3F) as u8;
                let b = (val & 0x1F) as u8;
                let r8 = (r << 3) | (r >> 2);
                let g8 = (g << 2) | (g >> 4);
                let b8 = (b << 3) | (b >> 2);
                decompressed[i] = u32::from_le_bytes([b8, g8, r8, 255]);
            }
            success = true;
        }
        8 => {
            for (i, chunk) in safe_image_data.chunks_exact(3).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], 255]);
            }
            success = true;
        }
        10 => {
            success = texture2ddecoder::decode_bc1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        11 => {
            let bw = (width + 3) / 4;
            let bh = (height + 3) / 4;
            if safe_image_data.len() >= bw * bh * 16 {
                for by in 0..bh {
                    for bx in 0..bw {
                        let offset = (by * bw + bx) * 16;
                        let alpha = &safe_image_data[offset..offset + 8];
                        let color = &safe_image_data[offset + 8..offset + 16];
                        let mut block = [0u32; 16];
                        if texture2ddecoder::decode_bc1(color, 4, 4, &mut block).is_ok() {
                            for i in 0..16 {
                                let px = bx * 4 + (i % 4);
                                let py = by * 4 + (i / 4);
                                if px < width && py < height {
                                    let a = (alpha[i / 2] >> ((i % 2) * 4)) & 0xF;
                                    let a8 = a | (a << 4);
                                    let c = block[i].to_le_bytes();
                                    decompressed[py * width + px] = u32::from_le_bytes([c[0], c[1], c[2], a8]);
                                }
                            }
                        }
                    }
                }
                success = true;
            }
        }
        12 => {
            success = texture2ddecoder::decode_bc3(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        14 => {
            for (i, chunk) in safe_image_data.chunks_exact(4).enumerate().take(width * height) {
                decompressed[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            success = true;
        }
        24 => {
            success = texture2ddecoder::decode_bc6(safe_image_data, aligned_w, aligned_h, &mut decompressed, false).is_ok();
        }
        25 => {
            success = texture2ddecoder::decode_bc7(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        26 => {
            success = texture2ddecoder::decode_bc4(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        27 => {
            success = texture2ddecoder::decode_bc5(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        30 | 31 => {
            success = texture2ddecoder::decode_pvrtc_2bpp(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        32 | 33 => {
            success = texture2ddecoder::decode_pvrtc_4bpp(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        34 | 60 | 61 => {
            success = texture2ddecoder::decode_etc1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        35 => {
            success = texture2ddecoder::decode_atc_rgb4(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        36 => {
            success = texture2ddecoder::decode_atc_rgba8(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        41 => {
            success = texture2ddecoder::decode_eacr(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        42 => {
            success = texture2ddecoder::decode_eacr_signed(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        43 => {
            success = texture2ddecoder::decode_eacrg(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        44 => {
            success = texture2ddecoder::decode_eacrg_signed(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        45 => {
            success = texture2ddecoder::decode_etc2_rgb(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        46 => {
            success = texture2ddecoder::decode_etc2_rgba1(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        47 => {
            success = texture2ddecoder::decode_etc2_rgba8(safe_image_data, aligned_w, aligned_h, &mut decompressed).is_ok();
        }
        48 | 54 | 66 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 4, 4, &mut decompressed).is_ok();
        }
        49 | 55 | 67 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 5, 5, &mut decompressed).is_ok();
        }
        50 | 56 | 68 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 6, 6, &mut decompressed).is_ok();
        }
        51 | 57 | 69 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 8, 8, &mut decompressed).is_ok();
        }
        52 | 58 | 70 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 10, 10, &mut decompressed).is_ok();
        }
        53 | 59 | 71 => {
            success = texture2ddecoder::decode_astc(safe_image_data, aligned_w, aligned_h, 12, 12, &mut decompressed).is_ok();
        }
        _ => {
            if safe_image_data.len() == width * height * 4 {
                for (i, chunk) in safe_image_data.chunks_exact(4).enumerate() {
                    decompressed[i] = u32::from_le_bytes([chunk[2], chunk[1], chunk[0], chunk[3]]);
                }
                success = true;
            }
        }
    }
    if success {
        let mut bytes = Vec::with_capacity(width * height * 4);
        let mut has_strong_alpha = false;
        let mut non_zero_count = 0;
        for y in (0..height).rev() {
            for x in 0..width {
                let idx = y * aligned_w + x;
                let p = decompressed[idx];
                let b = p.to_le_bytes();
                bytes.push(b[2]);
                bytes.push(b[1]);
                bytes.push(b[0]);
                bytes.push(b[3]);
                if b[3] > 50 {
                    has_strong_alpha = true;
                }
                if b[3] > 15 {
                    non_zero_count += 1;
                }
            }
        }
        let total_pixels = width * height;
        let threshold = total_pixels / 200;
        if !has_strong_alpha || non_zero_count < threshold {
            for chunk in bytes.chunks_exact_mut(4) {
                chunk[3] = 255;
            }
        }
        Some(bytes)
    } else {
        None
    }
}
fn compare_natural(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();
    loop {
        match (a_chars.peek(), b_chars.peek()) {
            (Some(a_c), Some(b_c)) => {
                if a_c.is_ascii_digit() && b_c.is_ascii_digit() {
                    let mut a_num = 0u64;
                    let mut a_len = 0;
                    while let Some(&c) = a_chars.peek() {
                        if let Some(digit) = c.to_digit(10) {
                            a_num = a_num.wrapping_mul(10).wrapping_add(digit as u64);
                            a_chars.next();
                            a_len += 1;
                        } else {
                            break;
                        }
                    }
                    let mut b_num = 0u64;
                    let mut b_len = 0;
                    while let Some(&c) = b_chars.peek() {
                        if let Some(digit) = c.to_digit(10) {
                            b_num = b_num.wrapping_mul(10).wrapping_add(digit as u64);
                            b_chars.next();
                            b_len += 1;
                        } else {
                            break;
                        }
                    }
                    if a_num != b_num {
                        return a_num.cmp(&b_num);
                    }
                    if a_len != b_len {
                        return a_len.cmp(&b_len);
                    }
                } else {
                    let ac = a_chars.next().unwrap();
                    let bc = b_chars.next().unwrap();
                    if ac != bc {
                        return ac.cmp(&bc);
                    }
                }
            }
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
        }
    }
}
fn get_model_base_name(filename: &str) -> String {
    let mut stem = filename.to_lowercase();
    for ext in &[".ab", ".asset", ".assets", ".assetbundle", ".bundle", ".bytes", ".prefab", ".unity3d", ".moc3", ".moc"] {
        if stem.ends_with(ext) {
            stem = stem[..stem.len() - ext.len()].to_string();
        }
    }
    for marker in &["l2d_", "live2d_", "spine_", "chara_", "character_"] {
        if let Some(idx) = stem.find(marker) {
            let start_digits = idx + marker.len();
            let mut end_digits = start_digits;
            while end_digits < stem.len() && stem.as_bytes()[end_digits].is_ascii_digit() {
                end_digits += 1;
            }
            if end_digits > start_digits {
                stem = stem[..end_digits].to_string();
                break;
            }
        }
    }
    let suffixes = &[
        "texture", "textures", "tex",
        "moc", "moc3",
        "physics", "physics3",
        "pose", "pose3",
        "motion", "motion3",
        "expression", "expressions", "exp", "exp3",
        "userdata", "userdata3",
        "cdi", "cdi3",
        "postprocess", "postprocessing",
        "material", "materials", "mat",
        "controller", "controllers",
        "animator", "animation", "animations",
        "prefab", "prefabs",
        "asset", "assets",
        "bundle", "bundles",
        "model", "model3"
    ];
    let mut changed = true;
    while changed {
        changed = false;
        if let Some(idx) = stem.rfind('_') {
            let part = &stem[idx + 1 ..];
            if part.len() >= 8 && part.chars().all(|c| c.is_ascii_hexdigit()) && !part.chars().all(|c| c.is_ascii_digit()) {
                stem = stem[..idx].to_string();
                changed = true;
                continue;
            }
        }
        if let Some(idx) = stem.rfind('_') {
            let part = &stem[idx + 1 ..];
            if !part.is_empty() && part.len() <= 3 && part.chars().all(|c| c.is_ascii_digit()) {
                stem = stem[..idx].to_string();
                changed = true;
                continue;
            }
        }
        for suffix in suffixes {
            let suffix_with_underscore = format!("_{}", suffix);
            if stem.ends_with(&suffix_with_underscore) {
                stem = stem[..stem.len() - suffix_with_underscore.len()].to_string();
                changed = true;
                break;
            }
            if stem.ends_with(suffix) {
                stem = stem[..stem.len() - suffix.len()].to_string();
                changed = true;
                break;
            }
        }
        if stem.ends_with('_') || stem.ends_with('.') || stem.ends_with('-') {
            stem.pop();
            changed = true;
        }
    }
    let sanitized = stem.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "");
    if sanitized.is_empty() {
        "model".to_string()
    } else {
        sanitized
    }
}
fn is_live2d_texture_name(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    if !name_lower.ends_with(".png") {
        return false;
    }
    let stem = &name_lower[..name_lower.len() - 4];
    if !stem.starts_with("texture") {
        return false;
    }
    let rest = if stem.starts_with("texture_") {
        &stem["texture_".len()..]
    } else {
        &stem["texture".len()..]
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}
fn reconstruct_live2d_models(output_dir: &Path) {
    let mut all_files = Vec::new();
    fn collect_all_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_all_files(&path, files);
                } else {
                    files.push(path);
                }
            }
        }
    }
    collect_all_files(output_dir, &mut all_files);
    let mut moc_files = Vec::new();
    for path in &all_files {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_lowercase();
            if ext_lower == "moc3" || ext_lower == "moc" {
                moc_files.push(path.clone());
            }
        }
    }
    if moc_files.is_empty() {
        return;
    }
    let mut processed_dirs = std::collections::HashSet::new();
    for moc_path in moc_files {
        let moc_filename = match moc_path.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        let base_key = get_model_base_name(&moc_filename);
        if base_key.is_empty() {
            continue;
        }
        let l2d_dir = output_dir.join(&base_key);
        processed_dirs.insert(l2d_dir.clone());
        if let Err(e) = std::fs::create_dir_all(&l2d_dir) {
            eprintln!("Failed to create directory '{}': {}", l2d_dir.display(), e);
            continue;
        }
        let dest_moc_path = l2d_dir.join(&moc_filename);
        if moc_path != dest_moc_path {
            if let Err(e) = std::fs::copy(&moc_path, &dest_moc_path) {
                eprintln!("Failed to copy moc file '{}': {}", moc_filename, e);
                continue;
            }
        }
        for path in &all_files {
            if path.starts_with(&l2d_dir) {
                continue;
            }
            if path == &moc_path {
                continue;
            }
            let rel_path = path.strip_prefix(output_dir).unwrap_or(path);
            let rel_path_str = rel_path.to_string_lossy().to_lowercase();
            if rel_path_str.contains(&base_key.to_lowercase()) {
                let filename = match path.file_name() {
                    Some(f) => f,
                    None => continue,
                };
                let filename_str = filename.to_string_lossy();
                let dest_path = if is_live2d_texture_name(&filename_str) {
                    let tex_dir = l2d_dir.join("textures");
                    let _ = std::fs::create_dir_all(&tex_dir);
                    tex_dir.join(filename)
                } else if filename_str.to_lowercase().ends_with(".motion3.json") || filename_str.to_lowercase().ends_with(".motion.json") {
                    let mot_dir = l2d_dir.join("motions");
                    let _ = std::fs::create_dir_all(&mot_dir);
                    mot_dir.join(filename)
                } else {
                    l2d_dir.join(filename)
                };
                if let Err(e) = std::fs::copy(path, &dest_path) {
                    eprintln!("Failed to copy asset from {:?} to {:?}: {}", path, dest_path, e);
                }
            }
        }
        let mut model_files = Vec::new();
        collect_all_files(&l2d_dir, &mut model_files);
        let mut textures = Vec::new();
        let mut physics = None;
        let mut display_info = None;
        let mut userdata = None;
        let mut pose = None;
        let mut expressions = Vec::new();
        let mut motions = std::collections::HashMap::new();
        for file_path in &model_files {
            let rel = file_path.strip_prefix(&l2d_dir).unwrap_or(file_path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let filename = match file_path.file_name().and_then(|f| f.to_str()) {
                Some(f) => f,
                None => continue,
            };
            if is_live2d_texture_name(filename) {
                textures.push(rel_str);
            } else {
                let filename_lower = filename.to_lowercase();
                if filename_lower.ends_with(".physics3.json") || filename_lower.ends_with(".physics.json") {
                    physics = Some(rel_str);
                } else if filename_lower.ends_with(".cdi3.json") || filename_lower.ends_with(".cdi.json") {
                    display_info = Some(rel_str);
                } else if filename_lower.ends_with(".userdata3.json") || filename_lower.ends_with(".userdata.json") {
                    userdata = Some(rel_str);
                } else if filename_lower.ends_with(".pose3.json") || filename_lower.ends_with(".pose.json") {
                    pose = Some(rel_str);
                } else if filename_lower.ends_with(".exp3.json") || filename_lower.ends_with(".exp.json") {
                    expressions.push(serde_json::json!({
                        "Name": filename.replace(".exp3.json", "").replace(".exp.json", ""),
                        "File": rel_str
                    }));
                } else if filename_lower.ends_with(".motion3.json") || filename_lower.ends_with(".motion.json") {
                    let group = motions.entry("".to_string()).or_insert_with(Vec::new);
                    group.push(serde_json::json!({
                        "File": rel_str
                    }));
                }
            }
        }
        textures.sort_unstable_by(|a, b| compare_natural(a, b));
        let moc_stem = moc_filename.strip_suffix(".moc3").or_else(|| moc_filename.strip_suffix(".moc")).unwrap_or(&moc_filename);
        let mut file_references = serde_json::json!({
            "Moc": moc_filename,
            "Textures": textures
        });
        if let Some(ref p) = physics {
            file_references["Physics"] = serde_json::Value::String(p.clone());
        }
        if let Some(ref d) = display_info {
            file_references["DisplayInfo"] = serde_json::Value::String(d.clone());
        }
        if let Some(ref u) = userdata {
            file_references["UserData"] = serde_json::Value::String(u.clone());
        }
        if let Some(ref p_pose) = pose {
            file_references["Pose"] = serde_json::Value::String(p_pose.clone());
        }
        if !expressions.is_empty() {
            file_references["Expressions"] = serde_json::Value::Array(expressions);
        }
        if !motions.is_empty() {
            file_references["Motions"] = serde_json::to_value(motions).unwrap_or(serde_json::Value::Null);
        }
        let model3_json = serde_json::json!({
            "Version": 3,
            "FileReferences": file_references
        });
        let model3_json_path = l2d_dir.join(format!("{}.model3.json", moc_stem));
        if let Ok(file) = std::fs::File::create(&model3_json_path) {
            let _ = serde_json::to_writer_pretty(file, &model3_json);
        }
    }
    let mut dirs_to_delete = std::collections::HashSet::new();
    for path in &all_files {
        if let Some(parent) = path.parent() {
            if parent == output_dir {
                continue;
            }
            let mut inside_processed = false;
            for dest_dir in &processed_dirs {
                if parent.starts_with(dest_dir) {
                    inside_processed = true;
                    break;
                }
            }
            if inside_processed {
                continue;
            }
            let rel = parent.strip_prefix(output_dir).unwrap_or(parent);
            let rel_str = rel.to_string_lossy().to_lowercase();
            for dest_dir in &processed_dirs {
                let base_key = dest_dir.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                if rel_str.contains(&base_key) {
                    dirs_to_delete.insert(parent.to_path_buf());
                }
            }
        }
    }
    for dir in dirs_to_delete {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
