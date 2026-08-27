use std::rc::Rc;

use jsonrpsee_wasm_client::{Client, WasmClientBuilder};
use oreak_core::HistoryEvent;
use oreak_protocol::{
    ApplyCommandRequest, ApplyCommandResponse, LevelHistoryRequest, LevelSnapshotResponse,
    LevelSubscriptionItem, MAX_LEVEL_HISTORY_PAGE_SIZE, OreakRpcClient as _, ProjectLevelTarget,
    UndoLatestRequest, UndoLatestResponse,
};
use yew::Callback;

const MAX_HYDRATED_HISTORY_EVENTS: usize = 4_096;

#[derive(Clone, Debug)]
pub enum RpcUpdate {
    Snapshot(LevelSnapshotResponse),
    History {
        target: ProjectLevelTarget,
        through_sequence: u64,
        events: Vec<HistoryEvent>,
    },
    Subscription(LevelSubscriptionItem),
    Closed(String),
}

#[derive(Clone)]
pub struct RpcClient {
    inner: Rc<Client>,
    endpoint: String,
}

impl RpcClient {
    pub async fn connect_same_origin(
        target: ProjectLevelTarget,
        updates: Callback<RpcUpdate>,
    ) -> Result<Self, String> {
        let endpoint = same_origin_endpoint()?;
        let inner = WasmClientBuilder::default()
            .build(&endpoint)
            .await
            .map_err(|error| error.to_string())?;
        let mut subscription = inner
            .subscribe_level(target.clone())
            .await
            .map_err(|error| format!("subscribe_level failed: {error}"))?;
        let initial = subscription
            .next()
            .await
            .ok_or_else(|| "level subscription closed before its initial snapshot".to_owned())?
            .map_err(|error| format!("initial level subscription item failed: {error}"))?;
        let LevelSubscriptionItem::Snapshot { snapshot } = initial else {
            return Err("level subscription did not start with a snapshot".to_owned());
        };
        let snapshot = *snapshot;
        if snapshot.target != target {
            return Err("level subscription snapshot referenced the wrong target".to_owned());
        }
        let events = fetch_history(&inner, &target, snapshot.server_sequence).await?;

        updates.emit(RpcUpdate::Snapshot(snapshot));
        updates.emit(RpcUpdate::History {
            target,
            through_sequence: events.last().map_or(0, |event| event.sequence),
            events,
        });
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(item) = subscription.next().await {
                match item {
                    Ok(item) => updates.emit(RpcUpdate::Subscription(item)),
                    Err(error) => {
                        updates.emit(RpcUpdate::Closed(format!(
                            "level subscription failed: {error}"
                        )));
                        return;
                    }
                }
            }
            updates.emit(RpcUpdate::Closed("level subscription closed".to_owned()));
        });

        Ok(Self {
            inner: Rc::new(inner),
            endpoint,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn apply_command(
        &self,
        request: ApplyCommandRequest,
    ) -> Result<ApplyCommandResponse, String> {
        self.inner
            .as_ref()
            .apply_command(request)
            .await
            .map_err(|error| format!("apply_command failed: {error}"))
    }

    pub async fn undo_latest(
        &self,
        request: UndoLatestRequest,
    ) -> Result<UndoLatestResponse, String> {
        self.inner
            .as_ref()
            .undo_latest(request)
            .await
            .map_err(|error| format!("undo_latest failed: {error}"))
    }
}

async fn fetch_history(
    client: &Client,
    target: &ProjectLevelTarget,
    through_sequence: u64,
) -> Result<Vec<HistoryEvent>, String> {
    if through_sequence > MAX_HYDRATED_HISTORY_EVENTS as u64 {
        return Err(format!(
            "level history exceeds the {MAX_HYDRATED_HISTORY_EVENTS}-event hydration safety cap"
        ));
    }

    let mut newest_first = Vec::with_capacity(through_sequence as usize);
    let mut before_sequence = through_sequence
        .checked_add(1)
        .ok_or_else(|| "level history cursor overflowed".to_owned())?;
    loop {
        let page = client
            .level_history(LevelHistoryRequest {
                target: target.clone(),
                before_sequence: Some(before_sequence),
                limit: MAX_LEVEL_HISTORY_PAGE_SIZE,
            })
            .await
            .map_err(|error| format!("level_history failed: {error}"))?;
        if page.target != *target {
            return Err("level_history returned the wrong target".to_owned());
        }
        if page
            .events
            .iter()
            .any(|event| event.sequence >= before_sequence)
            || page
                .events
                .windows(2)
                .any(|pair| pair[0].sequence <= pair[1].sequence)
        {
            return Err(
                "level_history returned events outside newest-first cursor order".to_owned(),
            );
        }
        if newest_first.len() + page.events.len() > MAX_HYDRATED_HISTORY_EVENTS {
            return Err(format!(
                "level history exceeds the {MAX_HYDRATED_HISTORY_EVENTS}-event hydration safety cap"
            ));
        }
        let oldest_sequence = page.events.last().map(|event| event.sequence);
        newest_first.extend(page.events);

        match (page.has_more, page.next_before_sequence) {
            (true, Some(next)) if oldest_sequence == Some(next) && next < before_sequence => {
                before_sequence = next;
            }
            (false, None) => break,
            _ => return Err("level_history returned an invalid continuation".to_owned()),
        }
    }

    newest_first.reverse();
    if newest_first.len() != through_sequence as usize
        || newest_first
            .iter()
            .enumerate()
            .any(|(index, event)| event.sequence != index as u64 + 1)
    {
        return Err(format!(
            "level_history did not cover every sequence through {through_sequence}"
        ));
    }
    Ok(newest_first)
}

fn same_origin_endpoint() -> Result<String, String> {
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
    let location = window.location();
    let protocol = location
        .protocol()
        .map_err(|_| "unable to read the page protocol".to_owned())?;
    let host = location
        .host()
        .map_err(|_| "unable to read the page host".to_owned())?;
    if host.is_empty() {
        return Err("RPC requires the app to be served over HTTP".to_owned());
    }

    let websocket_protocol = if protocol == "https:" { "wss" } else { "ws" };
    Ok(format!("{websocket_protocol}://{host}/rpc"))
}
