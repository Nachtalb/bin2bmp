use std::{
    env,
    fs::File,
    io::{self, BufWriter, Read, Write},
    process::ExitCode,
};

use bin2bmp::{write_bmp, Options};

const USAGE: &str = "\
Usage: bin2bmp [INPUT] [OUTPUT]

  INPUT   path to binary file, or '-' for stdin  (default: stdin)
  OUTPUT  path for .bmp output, or '-' for stdout (default: stdout)

Each byte becomes one grayscale pixel. Canvas is as square as possible.";

fn run(args: &[String]) -> io::Result<()> {
    let opts = Options::default();
    let mut paths = args.iter().map(String::as_str);
    let input = paths.next().filter(|p| *p != "-");
    let output = paths.next().filter(|p| *p != "-");
    if let Some(extra) = paths.next() {
        return Err(io::Error::other(format!("unexpected argument '{extra}'")));
    }

    // Only a regular file knows its length up front; anything else is buffered.
    let (input, len): (Box<dyn Read>, u64) = match input.map(File::open).transpose()? {
        Some(file) if file.metadata()?.is_file() => {
            let len = file.metadata()?.len();
            (Box::new(file), len)
        }
        other => {
            let mut buf = Vec::new();
            match other {
                Some(mut file) => file.read_to_end(&mut buf)?,
                None => io::stdin().lock().read_to_end(&mut buf)?,
            };
            let len = buf.len() as u64;
            (Box::new(io::Cursor::new(buf)), len)
        }
    };

    let out: Box<dyn Write> = match output {
        Some(path) => Box::new(File::create(path)?),
        None => Box::new(io::stdout().lock()),
    };
    let (width, height) = write_bmp(input, len, BufWriter::with_capacity(1 << 18, out), &opts)?;
    eprintln!("{width}x{height} ({len} bytes)");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
