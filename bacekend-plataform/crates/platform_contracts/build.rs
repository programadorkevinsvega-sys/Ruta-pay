fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    unsafe {
        std::env::set_var("PROTOC", protoc);
    }
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/platform_runtime.proto",
                "proto/identity_access.proto",
                "proto/tenant_registry.proto",
                "proto/realtime.proto",
                "proto/catalog_pricing.proto",
                "proto/entitlements_quotas.proto",
            ],
            &["proto"],
        )?;
    for proto in [
        "platform_runtime.proto",
        "identity_access.proto",
        "tenant_registry.proto",
        "realtime.proto",
        "catalog_pricing.proto",
        "entitlements_quotas.proto",
    ] {
        println!("cargo:rerun-if-changed=proto/{proto}");
    }
    Ok(())
}
