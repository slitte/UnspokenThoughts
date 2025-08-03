fn main() {
    prost_build::Config::new()
        .out_dir(std::env::var("OUT_DIR").unwrap())
        .compile_protos(
            &["proto/mesh.proto"],
            &["proto", "proto/meshtastic"], // ← beide Ordner als Include!
        )
        .unwrap();
}
