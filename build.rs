/*
 * This file is part of the rscap project.
 *
 * Build the Cap'n Proto schema file.
*/
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Compile the Cap'n Proto schema
    capnpc::CompilerCommand::new()
        .file("schema/stream_data.capnp")
        .run()
        .expect("Compiling Cap'n Proto schema");

    // Get the OUT_DIR environment variable
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // Define the path to the generated Cap'n Proto file
    let generated_path = Path::new(&out_dir).join("schema/stream_data_capnp.rs");

    // Define the destination path in the source directory
    let dest_path = Path::new("src/").join("stream_data_capnp.rs");

    // Copy the generated file to the destination path
    fs::copy(&generated_path, &dest_path).expect("Failed to copy generated Cap'n Proto file");

    // Optional: Output a message indicating the completion of the script
    println!("Copied generated Cap'n Proto file to {:?}", dest_path);
}
