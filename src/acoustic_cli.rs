#[tokio::main]
async fn main() -> std::process::ExitCode {
    #[cfg(unix)]
    {
        intellicord::acoustic_capture::cli().await
    }
    #[cfg(not(unix))]
    {
        eprintln!("PipeWire capture requires Linux/Unix");
        std::process::ExitCode::FAILURE
    }
}
