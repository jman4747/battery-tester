use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
	println!("cargo:rustc-link-search={}", env!("CARGO_MANIFEST_DIR"));
	Ok(())
}
