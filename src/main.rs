use std::{
    env,
    fs::File,
    io::{self, BufWriter, Read, Write},
    process::ExitCode,
};

use bin2bmp::{write_bmp, Mode, Options};

const USAGE: &str = "\
Usage: bin2bmp [OPTIONS] [INPUT] [OUTPUT]

  INPUT   path to binary file, or '-' for stdin  (default: stdin)
  OUTPUT  path for .bmp output, or '-' for stdout (default: stdout)

Options:
  -m, --mode MODE   what a pixel shows (default: bytes)
                      bytes  one pixel per byte, value = brightness
                      class  one pixel per byte: 0x00 black, 0xFF white,
                             printable ASCII blue, everything else red
                      entropy  one pixel per block, brightness = how random
                               the block is (compressed/encrypted = white)
                      digraph  256x256 plot of which byte follows which;
                               file formats have recognisable shapes
  -b, --block N     bytes per pixel in entropy mode (default: 256)
  -w, --width N     image width in pixels (default: as square as possible)
";

fn bad(msg: String) -> io::Error {
    io::Error::other(format!("{msg} (see --help)"))
}

fn num<T: std::str::FromStr>(opt: &str, value: &str) -> io::Result<T> {
    value
        .parse()
        .map_err(|_| bad(format!("{opt} needs a number, got '{value}'")))
}

fn run(args: &[String]) -> io::Result<()> {
    let mut opts = Options::default();
    let mut paths = Vec::new();
    let (mut mode, mut block) = ("bytes", 256);
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| bad(format!("{arg} needs a value")))
        };
        match arg {
            "-m" | "--mode" => mode = value()?,
            "-b" | "--block" => block = num(arg, value()?)?,
            "-w" | "--width" => opts.width = Some(num(arg, value()?)?),
            "-" => paths.push(arg),
            _ if arg.starts_with('-') => return Err(bad(format!("unknown option '{arg}'"))),
            _ => paths.push(arg),
        }
    }
    opts.mode = match mode {
        "bytes" => Mode::Bytes,
        "class" => Mode::Class,
        "entropy" => Mode::Entropy { block },
        "digraph" => Mode::Digraph,
        other => return Err(bad(format!("unknown mode '{other}'"))),
    };
    let mut paths = paths.into_iter();
    let input = paths.next().filter(|p| *p != "-");
    let output = paths.next().filter(|p| *p != "-");
    if let Some(extra) = paths.next() {
        return Err(bad(format!("unexpected argument '{extra}'")));
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
