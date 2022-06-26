mod github_processor;
mod job_processor;
mod sha;
mod table_builder;

use diffbot_lib::job::{
    runner::handle_jobs,
    types::{JobJournal, JobSender},
};
use octocrab::OctocrabBuilder;
use once_cell::sync::OnceCell;
use rocket::{figment::Figment, fs::FileServer, get, launch, routes};
use serde::Deserialize;
use std::{fs::File, io::Read, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[get("/")]
async fn index() -> &'static str {
    "IDB says hello!"
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub private_key_path: String,
    // TODO: use regular url and append /images in code
    pub file_hosting_url: String,
    pub app_id: u64,
    // TODO: Blacklist support
    // pub blacklist: Vec<u64>,
    // pub blacklist_contact: String,
}

static CONFIG: OnceCell<Config> = OnceCell::new();
// static FLAME_LAYER_GUARD: OnceCell<tracing_flame::FlushGuard<std::io::BufWriter<File>>> =
// OnceCell::new();

fn init_config(figment: &Figment) -> &Config {
    let config: Config = figment
        .extract()
        .expect("Missing config values in Rocket.toml");

    CONFIG.set(config).expect("Failed to set config");
    CONFIG.get().unwrap()
}

// fn init_global_subscriber() {
//     use tracing_subscriber::prelude::*;

//     let fmt_layer = tracing_subscriber::fmt::Layer::default();

//     let (flame_layer, guard) = tracing_flame::FlameLayer::with_file("./tracing.folded").unwrap();

//     tracing_subscriber::registry()
//         .with(fmt_layer)
//         .with(flame_layer)
//         .init();

//     FLAME_LAYER_GUARD
//         .set(guard)
//         .expect("Failed to store flame layer guard");
// }

fn read_key(path: PathBuf) -> Vec<u8> {
    let mut key_file =
        File::open(&path).unwrap_or_else(|_| panic!("Unable to find file {}", path.display()));

    let mut key = Vec::new();
    let _ = key_file
        .read_to_end(&mut key)
        .unwrap_or_else(|_| panic!("Failed to read key {}", path.display()));

    key
}

#[launch]
async fn rocket() -> _ {
    // init_global_subscriber();
    let rocket = rocket::build();
    let config = init_config(rocket.figment());

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("Octocrab failed to initialise");

    let journal = Arc::new(Mutex::new(
        JobJournal::from_file("jobs.json").await.unwrap(),
    ));

    tokio::fs::create_dir_all("./images").await.unwrap();

    let (job_sender, job_receiver) = flume::unbounded();
    let journal_clone = journal.clone();

    rocket::tokio::spawn(async move {
        handle_jobs(
            "IconDiffBot2",
            job_receiver,
            journal_clone,
            job_processor::do_job,
        )
        .await
    });

    rocket
        .manage(JobSender(job_sender))
        .manage(journal)
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
