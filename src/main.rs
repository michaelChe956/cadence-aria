#[tokio::main]
async fn main() {
    match cadence_aria::cli::run_cli_async(std::env::args().skip(1)).await {
        Ok(cadence_aria::cli::CliOutput::Text(text)) if !text.is_empty() => println!("{text}"),
        Ok(_) => {}
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            std::process::exit(1);
        }
    }
}
