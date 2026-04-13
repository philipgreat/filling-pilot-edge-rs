fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_path = "src/proto/fillingPilot.proto";
    let proto_dir = "src/proto";
    
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile(&[proto_path], &[proto_dir])?;
    Ok(())
}
