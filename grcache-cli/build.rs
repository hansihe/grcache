fn main() {
    protobuf_codegen::Codegen::new()
        .include("../proto")
        .inputs(["../proto/google/protobuf/descriptor.proto"])
        .cargo_out_dir("proto_gen")
        .run_from_script();
}
