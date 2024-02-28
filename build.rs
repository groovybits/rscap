use protobuf_codegen_pure::Codegen;

fn main() {
    Codegen::new()
        .out_dir("src/bin/")
        .inputs(&["schema/stream_data_proto.proto"])
        .include("schema/")
        .run()
        .expect("Running protoc failed.");
}
