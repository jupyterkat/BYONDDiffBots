use std::sync::Arc;

use diffbot_lib::{
    github::github_api::get_pull_files,
    github::{
        github_api::CheckRun,
        github_types::{Output, PullRequestEventPayload},
    },
    job::types::{Job, JobJournal, JobSender},
};
use octocrab::models::{pulls::FileDiff, InstallationId};
// use dmm_tools::dmi::IconFile;
use anyhow::Result;
use rocket::{
    http::Status,
    post,
    request::{FromRequest, Outcome},
    Request, State,
};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct GithubEvent(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubEvent {
    type Error = &'static str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("X-Github-Event") {
            Some(event) => Outcome::Success(GithubEvent(event.to_owned())),
            None => Outcome::Failure((Status::BadRequest, "Missing X-Github-Event header")),
        }
    }
}

async fn handle_pull_request(
    payload: PullRequestEventPayload,
    job_sender: &State<JobSender>,
    journal: &State<Arc<Mutex<JobJournal>>>,
) -> Result<()> {
    match payload.action.as_str() {
        "opened" => {}
        #[cfg(debug_assertions)]
        "reopened" => {}
        "synchronize" => {}
        _ => return Ok(()),
    }

    let check_run = CheckRun::create(
        &payload.pull_request.base.repo.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        Some("IconDiffBot2"),
    )
    .await?;

    if payload
        .pull_request
        .title
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PR title is None"))?
        .to_ascii_lowercase()
        .contains("[idb ignore]")
    {
        let output = Output {
            title: "PR Ignored".to_owned(),
            summary: "This PR has `[IDB IGNORE]` in the title. Aborting.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;
        return Ok(());
    }

    let files = get_pull_files(&payload.installation, &payload.pull_request).await?;

    let changed_dmis: Vec<FileDiff> = files
        .into_iter()
        .filter(|e| e.filename.ends_with(".dmi"))
        .collect();

    if changed_dmis.is_empty() {
        let output = Output {
            title: "No icon chages".to_owned(),
            summary: "There are no changed icon files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    check_run.mark_queued().await?;

    let pull = payload.pull_request;
    let installation = payload.installation;

    let job = Job {
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files: changed_dmis,
        check_run,
        installation: InstallationId(installation.id),
    };

    journal.lock().await.add_job(job.clone()).await;
    job_sender.0.send_async(job).await?;

    Ok(())
}

#[post("/payload", format = "json", data = "<payload>")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
    job_sender: &State<JobSender>,
    journal: &State<Arc<Mutex<JobJournal>>>,
) -> Result<&'static str, String> {
    // TODO: Check reruns
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    let payload: PullRequestEventPayload =
        serde_json::from_str(&payload).map_err(|e| format!("{e}"))?;

    handle_pull_request(payload, job_sender, journal)
        .await
        .map_err(|e| {
            eprintln!("Error handling event {e:?}");
            "An error occured while handling the event"
        })?;

    Ok("")
}
