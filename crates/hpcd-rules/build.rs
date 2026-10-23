use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let rules = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../rules");
    println!("cargo:rerun-if-changed={}", rules.display());

    let mut files = Vec::new();
    collect(&rules, &mut files);
    files.sort();

    let mut out = String::from("pub const RULES: &[(&str, &str)] = &[\n");
    for path in files {
        let rel: Vec<String> = path
            .strip_prefix(&rules)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let abs = fs::canonicalize(&path).unwrap();
        out += &format!("    ({:?}, include_str!({:?})),\n", rel.join("/"), abs);
    }
    out += "];\n";

    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("embedded_rules.rs");
    fs::write(dest, out).unwrap();
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if name != "tests" {
                collect(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "toml") {
            out.push(path);
        }
    }
}
