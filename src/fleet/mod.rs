pub mod codegen;
pub mod spec_model;
pub mod spec_parser;

use std::path::Path;

pub struct FleetOutput {
    pub source: String,
    pub files: Vec<(String, String)>,
}

/// Generate skeleton FORGE code from a plain-text system description.
pub fn generate(spec: &str) -> anyhow::Result<FleetOutput> {
    let model = spec_parser::parse_spec(spec);
    let source = codegen::generate(&model);

    // Self-validate: generated code must parse
    if let Err(e) = crate::parser::parse(&source) {
        anyhow::bail!("internal error: generated code failed to parse: {}", e);
    }

    let filename = format!("{}.forge", model.system_name);
    Ok(FleetOutput {
        files: vec![(filename, source.clone())],
        source,
    })
}

/// Write fleet output to a directory.
pub fn write_to_dir(output: &FleetOutput, dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (filename, content) in &output.files {
        let path = dir.join(filename);
        std::fs::write(&path, content)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
