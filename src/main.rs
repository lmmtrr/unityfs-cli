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
}
#[derive(Clone, Debug)]
struct ExtractFilter {
    extract_metadata: bool,
    filter_name: Option<String>,
    filter_type: Option<String>,
}
fn extract_bundle_files_in_parallel(files: &[PathBuf], base_output_dir: &Path, filter: &ExtractFilter) {
    let pb = indicatif::ProgressBar::new(0);
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
fn get_unique_path(dir: &Path, filename: &str) -> PathBuf {
    let base_path = dir.join(filename);
    if !base_path.exists() {
        return base_path;
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
            return new_path;
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
                    let cri_dir = output_dir.join("CriWare");
                    let _ = std::fs::create_dir_all(&cri_dir);
                    let dest = get_unique_path(&cri_dir, &filename);
                    if let Err(e) = std::fs::write(&dest, bytes) {
                        pb.println(format!("    Failed to write CriWare asset '{}': {}", dest.display(), e));
                        return false;
                    } else {
                        pb.println(format!("    Extracted CriWare {} file: {}", extension.to_uppercase(), dest.display()));
                        return true;
                    }
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
                    return find_and_extract_criware_bytes(&UnityValue::Bytes(bytes), output_dir, base_name, parent_key, pb);
                }
            }
            let mut extracted = false;
            for (idx, item) in arr.iter().enumerate() {
                let item_key = if parent_key.is_empty() {
                    idx.to_string()
                } else {
                    format!("{}_{}", parent_key, idx)
                };
                if find_and_extract_criware_bytes(item, output_dir, base_name, &item_key, pb) {
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
                if find_and_extract_criware_bytes(v, output_dir, base_name, &item_key, pb) {
                    extracted = true;
                }
            }
            extracted
        }
        _ => false,
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
    let bundle_output_dir = base_output_dir.to_path_buf();
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
    pb.inc_length(objects_to_extract.len() as u64);
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
                        extract_texture2d(&unity_value, &bundle_output_dir, &asset_manager, pb)
                    }
                    49 => {
                        extract_text_asset(&unity_value, &bundle_output_dir, pb)
                    }
                    43 => {
                        extract_mesh(&unity_value, &bundle_output_dir, pb)
                    }
                    83 => {
                        extract_audioclip(&unity_value, &bundle_output_dir, &asset_manager, pb)
                    }
                    48 => {
                        extract_shader(&unity_value, &bundle_output_dir, pb)
                    }
                    329 => {
                        extract_videoclip(&unity_value, &bundle_output_dir, &asset_manager, pb)
                    }
                    114 => {
                        let base_name = if !m_name.is_empty() {
                            m_name.clone()
                        } else {
                            format!("monobehaviour_{}", path_id)
                        };
                        let ext_success = find_and_extract_criware_bytes(&unity_value, &bundle_output_dir, &base_name, "", pb);
                        if filter.extract_metadata {
                            dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, path_id);
                        }
                        ext_success
                    }
                    1 | 4 | 21 | 74 | 115 => {
                        if filter.extract_metadata {
                            dump_asset_as_json(class_id, t_name, &unity_value, &bundle_output_dir, path_id)
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
            pb.inc(1);
            res
        })
        .collect();
}
fn extract_text_asset(val: &UnityValue, output_dir: &Path, pb: &indicatif::ProgressBar) -> bool {
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
        let safe_name = name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.', "");
        let filename = if safe_name.is_empty() {
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
        };
        let text_dir = output_dir.join("TextAsset");
        let _ = std::fs::create_dir_all(&text_dir);
        let dest = get_unique_path(&text_dir, &filename);
        if let Err(e) = std::fs::write(&dest, &data) {
            pb.println(format!("    Failed to write text asset '{}': {}", dest.display(), e));
            false
        } else {
            true
        }
    } else {
        false
    }
}
fn extract_texture2d(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
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
        let texture_dir = output_dir.join("Texture2D");
        let _ = std::fs::create_dir_all(&texture_dir);
        let dest = get_unique_path(&texture_dir, &filename);
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
fn extract_mesh(val: &UnityValue, output_dir: &Path, pb: &indicatif::ProgressBar) -> bool {
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
        let mesh_dir = output_dir.join("Mesh");
        let _ = std::fs::create_dir_all(&mesh_dir);
        let dest = get_unique_path(&mesh_dir, &filename);
        if let Err(e) = std::fs::write(&dest, obj_content) {
            pb.println(format!("    Failed to write Mesh OBJ '{}': {}", dest.display(), e));
            false
        } else {
            true
        }
    } else {
        false
    }
}
fn extract_audioclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
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
    let audio_dir = output_dir.join("AudioClip");
    let _ = std::fs::create_dir_all(&audio_dir);
    let dest = get_unique_path(&audio_dir, &filename);
    if let Err(e) = std::fs::write(&dest, &audio_data) {
        pb.println(format!("    Failed to write AudioClip '{}': {}", dest.display(), e));
        false
    } else {
        true
    }
}
fn extract_videoclip(
    val: &UnityValue,
    output_dir: &Path,
    asset_manager: &AssetManager,
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
            let video_dir = output_dir.join("VideoClip");
            let _ = std::fs::create_dir_all(&video_dir);
            let dest = get_unique_path(&video_dir, &filename);
            if let Err(e) = std::fs::write(&dest, &video_data) {
                pb.println(format!("    Failed to write VideoClip '{}': {}", dest.display(), e));
                false
            } else {
                true
            }
        } else {
            pb.println(format!("    Failed to extract VideoClip raw bytes: resource data is missing or empty."));
            false
        }
    } else {
        false
    }
}
fn extract_shader(val: &UnityValue, output_dir: &Path, pb: &indicatif::ProgressBar) -> bool {
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
        let shader_dir = output_dir.join("Shader");
        let _ = std::fs::create_dir_all(&shader_dir);
        let dest = get_unique_path(&shader_dir, &filename);
        if let Err(e) = std::fs::write(&dest, &data) {
            pb.println(format!("    Failed to write Shader '{}': {}", dest.display(), e));
            false
        } else {
            true
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
    let target_dir = output_dir.join(sub_dir_name);
    let _ = std::fs::create_dir_all(&target_dir);
    let dest = get_unique_path(&target_dir, &filename);
    if let Ok(json_str) = serde_json::to_string_pretty(&json_val) {
        std::fs::write(&dest, json_str).is_ok()
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
