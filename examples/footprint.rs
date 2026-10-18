use poznan::{LookupParser, ScheduleParser};

fn main() {
    println!("target pointer width: {} bits", usize::BITS);
    println!(
        "ScheduleParser state: {} bytes",
        size_of::<ScheduleParser<'_>>()
    );
    println!(
        "LookupParser state: {} bytes",
        size_of::<LookupParser<'_>>()
    );
    println!(
        "schedule caller buffers: {} bytes (tag 512 + text 256 + metadata 512)",
        512 + 256 + 512
    );
    println!(
        "lookup caller buffers: {} bytes (tag 512 + text 256)",
        512 + 256
    );
    println!("body read buffer in CLI: 1024 bytes");
    println!(
        "MobilityParser state: {} bytes",
        size_of::<poznan::mobility::Parser<'_>>()
    );
    println!("mobility caller buffers: 2560 bytes (feature 2048 + strings 512)");
    println!(
        "ABI storage sizes only; excludes call stack, output buffering, HTTP/TLS, process and radio memory."
    );
}
