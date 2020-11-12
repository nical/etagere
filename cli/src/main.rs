extern crate etagere;
#[macro_use]
extern crate serde;

use etagere::*;
use etagere::euclid::size2;
use clap::*;

use std::io::prelude::*;
use std::fs::{File, OpenOptions};

#[derive(Serialize, Deserialize)]
struct Session {
    atlas: AtlasAllocator,
    names: std::collections::HashMap<String, Allocation>,
    next_id: u32,
}

fn main() {
    let matches = App::new("Étagère command-line interface")
        .version("0.1")
        .author("Nicolas Silva <nical@fastmail.com>")
        .about("Dynamic texture atlas allocator.")
        .subcommand(
            SubCommand::with_name("init")
            .about("Initialize the atlas")
            .arg(Arg::with_name("WIDTH")
                .help("Rectangle width.")
                .value_name("WIDTH")
                .takes_value(true)
                .required(true)
            )
            .arg(Arg::with_name("HEIGHT")
                .help("Rectangle height.")
                .value_name("HEIGHT")
                .takes_value(true)
                .required(true)
            )
            .arg(Arg::with_name("VERTICAL_SHELVES")
                .long("vertical-shelves")
                .help("Use vertical instead of horizontal shelves.")
                .value_name("VERTICAL_SHELVES")
                .takes_value(false)
                .required(false)
            )
            .arg(Arg::with_name("ALIGN_X")
                .long("align-x")
                .help("Round up the width of the allocated rectangle to a multiple of the provided value.")
                .value_name("ALIGN_X")
                .takes_value(true)
                .required(false)
            )
            .arg(Arg::with_name("ALIGN_Y")
                .long("align-y")
                .help("Round up the width of the allocated rectangle to a multiple of the provided value.")
                .value_name("ALIGN_Y")
                .takes_value(true)
                .required(false)
            )
            .arg(Arg::with_name("ATLAS")
                .short("a")
                .long("atlas")
                .help("Sets the output atlas file to use")
                .value_name("FILE")
                .takes_value(true)
                .required(false)
            )
            .arg(Arg::with_name("SVG_OUTPUT")
                .long("svg")
                .help("Dump the atlas in an SVG file")
                .value_name("SVG_OUTPUT")
                .takes_value(true)
                .required(false)
            )
        )
        .subcommand(
            SubCommand::with_name("allocate")
            .about("Allocate a rectangle")
            .arg(Arg::with_name("WIDTH")
                .help("Rectangle width.")
                .value_name("WIDTH")
                .takes_value(true)
                .required(true)
            )
            .arg(Arg::with_name("HEIGHT")
                .help("Rectangle height.")
                .value_name("HEIGHT")
                .takes_value(true)
                .required(true)
            )
            .arg(Arg::with_name("NAME")
                .short("-n")
                .long("name")
                .help("Set a name to identify the rectangle.")
                .value_name("NAME")
                .takes_value(true)
                .required(false)
             )
            .arg(Arg::with_name("ATLAS")
                .short("a")
                .long("atlas")
                .help("Sets the output atlas file to use")
                .value_name("FILE")
                .takes_value(true)
                .required(false)
            )
            .arg(Arg::with_name("SVG_OUTPUT")
                .long("svg")
                .help("Dump the atlas in an SVG file")
                .value_name("SVG_OUTPUT")
                .takes_value(true)
                .required(false)
            )
        )
        .subcommand(
            SubCommand::with_name("deallocate")
            .about("De-allocate a rectangle")
            .arg(Arg::with_name("NAME")
                .help("Name of the rectangle to remove.")
                .value_name("NAME")
                .takes_value(true)
                .required(true)
             )
            .arg(Arg::with_name("ATLAS")
                .short("a")
                .long("atlas")
                .help("Sets the output file to use")
                .value_name("FILE")
                .takes_value(true)
                .required(false)
            )
            .arg(Arg::with_name("SVG_OUTPUT")
                .long("svg")
                .help("Dump the atlas in an SVG file")
                .value_name("SVG_OUTPUT")
                .takes_value(true)
                .required(false)
            )
        )
        .subcommand(
            SubCommand::with_name("svg")
            .about("Dump the atlas as SVG")
            .arg(Arg::with_name("ATLAS")
                .short("-a")
                .long("atlas")
                .help("Input texture atlas file.")
                .value_name("ATLAS")
                .takes_value(true)
             )
            .arg(Arg::with_name("SVG_OUTPUT")
                .help("Output SVG file to use")
                .value_name("FILE")
                .takes_value(true)
                .required(false)
            )
        )
        .subcommand(
            SubCommand::with_name("list")
            .about("List the allocations and free rectangles in the atlas")
            .arg(Arg::with_name("ATLAS")
                .short("-a")
                .long("atlas")
                .help("Input texture atlas file.")
                .value_name("ATLAS")
                .takes_value(true)
             )
        )
        .get_matches();

    if let Some(cmd) = matches.subcommand_matches("init") {
        init(&cmd);
    } else if let Some(cmd) = matches.subcommand_matches("allocate") {
        allocate(&cmd);
    } else if let Some(cmd) = matches.subcommand_matches("deallocate") {
        deallocate(&cmd);
    } else if let Some(cmd) = matches.subcommand_matches("svg") {
        svg(&cmd);
    } else if let Some(cmd) = matches.subcommand_matches("list") {
        list(&cmd);
    }
}

fn read_atlas(args: &ArgMatches) -> Session {
    let atlas_file_name = args.value_of("ATLAS").unwrap_or("atlas.ron");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(atlas_file_name)
        .expect(
            "Failed to open the atlas file."
        );

    ron::de::from_reader(file).expect("Failed to parse the atlas")
}

fn write_atlas(session: &Session, args: &ArgMatches) {
    let serialized: String = ron::ser::to_string_pretty(
        &session,
        ron::ser::PrettyConfig::default(),
    ).unwrap();

    let atlas_file_name = args.value_of("ATLAS").unwrap_or("atlas.ron");
    let mut atlas_file = std::fs::File::create(atlas_file_name).expect(
        "Failed to open the atlas file."
    );

    atlas_file.write_all(serialized.as_bytes()).expect(
        "Failed to write into the atlas file."
    );
}

fn init(args: &ArgMatches) {
    let w = args.value_of("WIDTH").expect("Missing width.").parse::<i32>().unwrap();
    let h = args.value_of("HEIGHT").expect("Missing height.").parse::<i32>().unwrap();

    let default_options = etagere::DEFAULT_OPTIONS;

    let options = etagere::AllocatorOptions {
        alignment: size2(
            args.value_of("ALIGN_X")
                .map(|s| s.parse::<i32>().unwrap())
                .unwrap_or(default_options.alignment.width),
            args.value_of("ALIGN_Y")
                .map(|s| s.parse::<i32>().unwrap())
                .unwrap_or(default_options.alignment.width),
        ),
        vertical_shelves: args.is_present("VERTICAL_SHELVES"),
    };

    let session = Session {
        atlas: AtlasAllocator::with_options(size2(w, h), &options),
        names: std::collections::HashMap::default(),
        next_id: 0,
    };

    write_atlas(&session, &args);

    if args.is_present("SVG_OUTPUT") {
        svg(args);
    }
}



fn allocate(args: &ArgMatches) {
    let mut session = read_atlas(args);

    let w = args.value_of("WIDTH").expect("Missing width.").parse::<i32>().unwrap();
    let h = args.value_of("HEIGHT").expect("Missing height.").parse::<i32>().unwrap();

    let alloc = session.atlas.allocate(size2(w, h));

    if alloc.is_none() {
        eprintln!("Allocation of size {}x{} failed.", w, h);
        return;
    }

    let alloc = alloc.unwrap();

    let name = args.value_of("NAME").map(|name| name.to_string()).unwrap_or_else(|| {
        session.next_id += 1;
        format!("#{}", session.next_id)
    });

    let r = alloc.rectangle;

    println!(
        "Allocated rectangle {} of size {}x{} at origin [{}, {}]",
        name, r.size().width, r.size().height, r.min.x, r.min.y,
    );

    if let Some(old) = session.names.insert(name.clone(), alloc) {
        println!("Previous allocation with name {:?} was deallocated.", name);
        session.atlas.deallocate(old.id);
    }

    write_atlas(&session, args);

    if args.is_present("SVG_OUTPUT") {
        svg(args);
    }
}

fn deallocate(args: &ArgMatches) {
    let mut session = read_atlas(args);

    let name = args.value_of("NAME").expect("Need a rectangle name");
    let id = session.names.remove(name).unwrap().id;

    session.atlas.deallocate(id);

    write_atlas(&session, args);

    if args.is_present("SVG_OUTPUT") {
        svg(args);
    }
}

fn svg(args: &ArgMatches) {
    let session = read_atlas(args);

    let svg_file_name = args.value_of("SVG_OUTPUT").unwrap_or("atlas.svg");
    let mut svg_file = File::create(svg_file_name).expect(
        "Failed to open the SVG file."
    );

    etagere::dump_svg(&session.atlas, &mut svg_file).expect(
        "Failed to write into the SVG file."
    );
}

fn list(args: &ArgMatches) {
    let session = read_atlas(args);

    println!("# Allocated rectangles");
    for (name, &alloc) in &session.names {
        let rect = alloc.rectangle;
        println!(
            " - {}: size {}x{} at origin [{}, {}]",
            name, rect.size().width, rect.size().height, rect.min.x, rect.min.y
        );

        break;
    }
}

