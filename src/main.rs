use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match teatro::cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fatal application error: {error}");
            ExitCode::FAILURE
        }
    }
}
