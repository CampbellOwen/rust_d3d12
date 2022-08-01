use anyhow::Result;

fn main() -> Result<()> {
    let out_dir = std::env::var("OUT_DIR")? + "../../../../";

    std::fs::copy("dxcompiler.dll", format!("{}dxcompiler.dll", &out_dir))?;
    println!("!cargo:rerun-if-changed=dxcompiler.dll",);
    std::fs::copy("dxil.dll", format!("{}dxil.dll", &out_dir))?;
    println!("!cargo:rerun-if-changed=dxil.dll",);

    for entry in std::fs::read_dir("src/shaders")? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "hlsl" {
                    let out_path = std::path::Path::new(&out_dir).join(path.file_name().unwrap());
                    std::fs::copy(&path, &out_path)?;
                    println!(
                        "!cargo:rerun-if-changed=src/shaders/{}",
                        path.file_name().unwrap().to_str().unwrap()
                    );
                }
            }
        }
    }

    Ok(())
}
