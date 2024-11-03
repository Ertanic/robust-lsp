use std::sync::Arc;
use tower_lsp::{
    lsp_types::{
        self, notification::Progress, request::WorkDoneProgressCreate, NumberOrString,
        ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin,
        WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkDoneProgressReport,
    },
    Client,
};
use tracing::instrument;

pub fn check_project_compliance(params: &lsp_types::InitializeParams) -> bool {
    if let Some(root_uri) = params.root_uri.as_ref() {
        let root_path = root_uri.to_file_path().unwrap();

        return root_path.join("SpaceStation14.sln").exists()
            || root_path.join("RobustToolbox/RobustToolbox.sln").exists();
    }

    false
}

#[derive(Default, Debug)]
pub struct ProgressStatusInit {
    pub id: String,
    pub title: String,
    pub cancellable: bool,
    pub first_message: Option<String>,
    pub percentage: u32,
}

pub struct ProgressStatus {
    pub id: String,
    client: Arc<Client>,
    percentage: u32,
}

impl ProgressStatus {
    #[instrument(skip(client), fields(id = %id.as_ref()))]
    pub async fn new(client: Arc<Client>, id: impl AsRef<str>) -> Self {
        let id = id.as_ref();

        assert!(!id.trim().is_empty());

        tracing::trace!("Created new progress status");

        client
            .send_request::<WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: NumberOrString::String(id.to_owned()),
            })
            .await
            .unwrap();

        Self {
            id: id.to_owned(),
            percentage: 0,
            client,
        }
    }

    #[instrument(skip_all)]
    pub async fn new_with(client: Arc<Client>, params: ProgressStatusInit) -> Self {
        assert!(!params.id.trim().is_empty());
        assert!(!params.title.trim().is_empty());

        tracing::trace!("Initialized progress status with params: {:#?}", params);

        let instance = Self::new(client, params.id).await;

        instance
            .client
            .send_notification::<Progress>(ProgressParams {
                token: NumberOrString::String(instance.id.clone()),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                    WorkDoneProgressBegin {
                        title: params.title,
                        cancellable: Some(params.cancellable),
                        message: params.first_message,
                        percentage: Some(params.percentage),
                    },
                )),
            })
            .await;

        instance
    }

    #[instrument(skip(self))]
    pub async fn increment(&mut self) {
        self.next_state(self.percentage + 1, None).await;
    }

    #[instrument(skip(self), fields(id = %self.id, percentage = self.percentage))]
    pub async fn next_state(&mut self, next_percentage: u32, next_message: Option<String>) {
        self.percentage = next_percentage;

        self.client
            .send_notification::<Progress>(ProgressParams {
                token: NumberOrString::String(self.id.clone()),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Report(
                    WorkDoneProgressReport {
                        cancellable: Some(true),
                        message: next_message,
                        percentage: Some(next_percentage),
                    },
                )),
            })
            .await;
    }

    #[instrument(skip(self), fields(id = %self.id))]
    pub async fn finish(&self, message: Option<&str>) {
        tracing::trace!("Finishing progress status.");

        self.client
            .send_notification::<Progress>(ProgressParams {
                token: NumberOrString::String(self.id.clone()),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd {
                    message: message.map(|s| s.to_string()),
                })),
            })
            .await;
    }
}

#[inline(always)]
pub fn percentage(actual: u32, max: u32) -> u32 {
    (100 as f64 * actual as f64 / max as f64).round() as u32
}