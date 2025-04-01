use std::{future::Future, sync::Arc};
use tower_lsp::{
    lsp_types::{
        notification::Progress, request::WorkDoneProgressCreate, InitializeParams, NumberOrString,
        Position, ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin,
        WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkDoneProgressReport,
    },
    Client,
};
use tracing::instrument;

pub fn check_project_compliance(params: &InitializeParams) -> bool {
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
    (100f64 * actual as f64 / max as f64).round() as u32
}

pub fn block<Fn, F>(func: Fn) -> F::Output
where
    Fn: FnOnce() -> F,
    F: Future,
{
    tokio::task::block_in_place(move || {
        tokio::runtime::Handle::current().block_on(async move { func().await })
    })
}

// Calculate the position for the correct node search.
// P.S. Why on tree-sitter playground everything works correctly (in javascript)
// even without dancing with tambourine - idk.
pub fn get_columns(position: Position, src: &str) -> (usize, usize) {
    let line = src.lines().nth(position.line as usize).unwrap_or_default();

    // If the string is empty, we use the cursor coordinates
    // and minus them by one, otherwise the root node `stream` will be searched.
    let trim_str = line.trim();
    if trim_str.len() == 0 {
        let col = if position.character == 0 {
            position.character
        } else {
            position.character - 1
        } as usize;

        (col, col)

    // If the string starts with `-`, we try to find the coordinate starting before
    // the `-` character, since only there tree-sitter can detect the `block_sequence_item` node.
    } else if trim_str.len() == 1 && trim_str.chars().all(|c| c == '-') {
        let mut col = 0;
        let chars = line.chars();
        for ch in chars {
            if ch == '-' {
                break;
            }
            col += 1;
        }
        (col, col)

    // If the string is not empty, we catch the beginning of the text
    // and the end of the text to properly search for child nodes.
    } else {
        let mut scol = line.chars().count();
        let mut ecol = scol;
        let mut chars = {
            let mut c = line.chars();
            while let Some(_) = c.next_back() {
                scol -= 1;

                if scol == position.character as usize {
                    break;
                } else if scol < position.character as usize {
                    c.next();
                    scol += 1;
                    break;
                }
            }
            c
        };
        let mut text = false;
        while let Some(ch) = chars.next_back() {
            scol -= 1;

            if !ch.is_whitespace() {
                text = true;
            } else if text && ch.is_whitespace() {
                break;
            }

            if !text {
                ecol -= 1;
            }
        }

        (scol + 1, ecol - 1)
    }
}
