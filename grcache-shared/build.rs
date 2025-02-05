use protobuf_codegen::Codegen;

fn main() {
    Codegen::new()
        .protoc()
        .cargo_out_dir("generated")
        .input("../proto/grcache/options.proto")
        .include("../proto")
        .run_from_script();
}
