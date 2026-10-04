use anyhow::{Context, Result};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let source = PathBuf::from(
        args.get(1)
            .context("用法：ocs-migrate LEGACY_CONFIG RUST_DATA RESOURCES")?,
    );
    let dest = PathBuf::from(args.get(2).context("缺少目标数据目录")?);
    let resources = PathBuf::from(args.get(3).context("缺少资源目录")?);
    let store = ocs_core::storage::Storage::open(dest, resources)?;
    println!("{}", ocs_core::migration::import_legacy(&store, &source)?);
    Ok(())
}
