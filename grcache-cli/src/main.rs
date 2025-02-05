use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

use clap::Parser;
use serde::Serialize;

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Validates the state of the cluster, with optional substitutions
    ValidateCluster {},
    Validate {
        #[arg(long)]
        descriptor_sets: Vec<PathBuf>,
    },
    UploadDescriptors {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        bucket_id: String,
        #[arg(long)]
        descriptor_set: PathBuf,
    },
    /// Generates schemas and CRDs to files
    GenerateSchemas { out_dir: PathBuf },
}

fn main() {
    let args = Args::parse();

    match args.command {
        Commands::ValidateCluster {} => todo!(),
        Commands::Validate { descriptor_sets } => todo!(),
        Commands::UploadDescriptors {
            config,
            bucket_id,
            descriptor_set,
        } => todo!(),
        Commands::GenerateSchemas { out_dir } => {
            //fn write_json<T: Serialize>(out_dir: &Path, name: &str, value: T) {
            //    let mut path = PathBuf::new();
            //    path.push(out_dir);
            //    path.push(name);

            //    let file = File::create(&path).unwrap();
            //    serde_json::to_writer(BufWriter::new(file), &value).unwrap();
            //}

            fn write_yaml<T: Serialize>(out_dir: &Path, name: &str, value: T) {
                let mut path = PathBuf::new();
                path.push(out_dir);
                path.push(name);

                let file = File::create(&path).unwrap();
                serde_yaml::to_writer(BufWriter::new(file), &value).unwrap();
            }

            write_yaml(
                &out_dir,
                "grcache_service_crd.yaml",
                grcache_shared::config::crd::GrcacheService::crd(),
            );
        }
    }

    //Commands::GenerateSpec { in_file } => {
    //    let mut file = File::open(in_file).expect("failed to open input file");
    //    let mut buf = Vec::new();
    //    file.read_to_end(&mut buf).unwrap();

    //    let file_descriptor_set =
    //        protos::descriptor::FileDescriptorSet::parse_from_bytes(&buf).unwrap();

    //    for file_descr in file_descriptor_set.file.iter() {
    //        if file_descr.service.len() > 0 {
    //            println!("{:#?}", file_descr);
    //        }
    //    }
    //}
    //Commands::GenerateUploadSpec { config, in_file } => todo!(),

    //match args.command {
    //    Commands::UploadDescriptorSet {
    //        config_path,
    //        bucket_id,
    //        descriptor_set_path,
    //    } => {
    //        //let mut
    //        // todo
    //    }
    //}
}
