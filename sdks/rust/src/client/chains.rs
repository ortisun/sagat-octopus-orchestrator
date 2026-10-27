use super::*;
#[derive(Clone, Debug)]
pub struct ChainCommand {
    pub expected_run_id: String,
}
#[derive(Clone)]
pub struct ChainQueryRequest {
    pub args: Value,
    pub expected_run_id: Option<String>,
}
#[derive(Clone)]
pub struct ChainUpdateRequest {
    pub update_key: String,
    pub args: Value,
    pub expected_state_revision: Option<u64>,
    pub expected_run_id: Option<String>,
}
impl std::fmt::Debug for ChainQueryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainQueryRequest")
            .field("has_expected_head", &self.expected_run_id.is_some())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for ChainUpdateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainUpdateRequest")
            .field("has_expected_head", &self.expected_run_id.is_some())
            .field("expected_state_revision", &self.expected_state_revision)
            .finish_non_exhaustive()
    }
}
fn path<'a>(namespace: &'a str, chain: &'a str) -> Result<Vec<&'a str>, SagatError> {
    uuid(namespace)?;
    uuid(chain)?;
    Ok(vec!["namespaces", namespace, "chains", chain])
}
fn chain_page(raw: Value, cursor: Option<&str>) -> Result<crate::ChainRunPage, SagatError> {
    if raw.get("nextCursor").is_none() {
        return Err(protocol());
    }
    let page = crate::types::parse_page(raw, crate::ChainRun::parse)?;
    if page.next_cursor.is_some() && page.next_cursor.as_deref() == cursor {
        return Err(protocol());
    }
    Ok(page)
}
impl Client {
    pub async fn inspect_chain(
        &self,
        namespace: &str,
        chain: &str,
        call: CallOptions,
    ) -> Result<crate::ChainInspection, SagatError> {
        self.execute(
            WireRequest::new(Method::GET, &path(namespace, chain)?, 200),
            call,
        )
        .await?
        .parse(crate::ChainInspection::parse)
    }
    pub async fn list_chain_runs(
        &self,
        namespace: &str,
        chain: &str,
        page: PageOptions,
        call: CallOptions,
    ) -> Result<crate::ChainRunPage, SagatError> {
        let mut path = path(namespace, chain)?;
        path.push("runs");
        let request = WireRequest::new(Method::GET, &path, 200)
            .query(page_query(page.limit, page.cursor.as_deref())?);
        self.execute(request, call)
            .await?
            .parse(|raw| chain_page(raw, page.cursor.as_deref()))
    }
    pub fn list_all_chain_runs(
        &self,
        namespace: &str,
        chain: &str,
        page: PageOptions,
        call: CallOptions,
    ) -> impl futures_util::Stream<Item = Result<crate::ChainRun, SagatError>> + Send + 'static
    {
        futures_util::stream::try_unfold(
            (
                self.clone(),
                namespace.to_owned(),
                chain.to_owned(),
                page,
                call,
                std::collections::VecDeque::<crate::ChainRun>::new(),
                false,
            ),
            |(client, namespace, chain, mut page, call, mut buffer, mut finished)| async move {
                loop {
                    if call.cancel.is_cancelled() {
                        return Err(cancelled());
                    }
                    if let Some(item) = buffer.pop_front() {
                        return Ok(Some((
                            item,
                            (client, namespace, chain, page, call, buffer, finished),
                        )));
                    }
                    if finished {
                        return Ok(None);
                    }
                    let response = client
                        .list_chain_runs(&namespace, &chain, page.clone(), call.clone())
                        .await?;
                    finished = response.next_cursor.is_none();
                    page.cursor = response.next_cursor;
                    buffer = response.items.into();
                }
            },
        )
    }
    pub async fn query_chain(
        &self,
        namespace: &str,
        chain: &str,
        name: &str,
        query: ChainQueryRequest,
        call: CallOptions,
    ) -> Result<crate::QueryResult, SagatError> {
        let mut path = path(namespace, chain)?;
        operation_name(name)?;
        path.extend(["queries", name]);
        let mut body = json!({"args":query.args});
        if let Some(head) = query.expected_run_id {
            uuid(&head)?;
            body["expectedRunId"] = json!(head);
        }
        self.execute(
            WireRequest::new(Method::POST, &path, 200)
                .json(&body)?
                .repeatable(),
            call,
        )
        .await?
        .parse(crate::QueryResult::parse)
    }
    pub async fn update_chain(
        &self,
        namespace: &str,
        chain: &str,
        name: &str,
        update: ChainUpdateRequest,
        call: CallOptions,
    ) -> Result<crate::UpdateReceipt, SagatError> {
        let mut path = path(namespace, chain)?;
        operation_name(name)?;
        path.extend(["updates", name]);
        if !body_key(&update.update_key, 128)
            || update
                .expected_state_revision
                .is_some_and(|v| v > i64::MAX as u64)
        {
            return Err(validation());
        }
        let mut body = json!({"updateKey":update.update_key,"args":update.args});
        if let Some(head) = update.expected_run_id {
            uuid(&head)?;
            body["expectedRunId"] = json!(head);
        }
        if let Some(revision) = update.expected_state_revision {
            body["expectedStateRevision"] = json!(revision);
        }
        self.execute(
            WireRequest::new(Method::POST, &path, 200)
                .json(&body)?
                .repeatable(),
            call,
        )
        .await?
        .parse(crate::UpdateReceipt::parse)
    }
    pub async fn signal_chain(
        &self,
        namespace: &str,
        chain: &str,
        name: &str,
        payload: &Value,
        command: ChainCommand,
        call: CallOptions,
    ) -> Result<SignalResult, SagatError> {
        let mut path = path(namespace, chain)?;
        path_name(name)?;
        uuid(&command.expected_run_id)?;
        path.extend(["signals", name]);
        self.execute(
            WireRequest::new(Method::POST, &path, 202)
                .json(&json!({"expectedRunId":command.expected_run_id,"payload":payload}))?
                .keyed(key(&call)?),
            call,
        )
        .await?
        .parse(SignalResult::parse)
    }
    pub async fn cancel_chain(
        &self,
        namespace: &str,
        chain: &str,
        reason: &str,
        command: ChainCommand,
        call: CallOptions,
    ) -> Result<(), SagatError> {
        let mut path = path(namespace, chain)?;
        uuid(&command.expected_run_id)?;
        if reason.is_empty() || reason.chars().count() > 512 {
            return Err(validation());
        }
        path.push("cancel");
        self.execute(
            WireRequest::new(Method::POST, &path, 202)
                .json(&json!({"expectedRunId":command.expected_run_id,"reason":reason}))?
                .keyed(key(&call)?),
            call,
        )
        .await
        .map(|_| ())
    }
    pub async fn wait_for_chain_completion(
        &self,
        namespace: &str,
        chain: &str,
        options: WaitOptions,
    ) -> Result<crate::ChainInspection, SagatError> {
        path(namespace, chain)?;
        if !valid_timeout(options.timeout) || !valid_timeout(options.poll_interval) {
            return Err(validation());
        }
        let wait = async {
            loop {
                let chain = self
                    .inspect_chain(namespace, chain, options.call.clone())
                    .await?;
                if chain.final_outcome.is_some() {
                    return Ok(chain);
                }
                let factor = 0.8 + f64::from(Uuid::new_v4().as_bytes()[0]) * 0.4 / 255.0;
                tokio::select! {biased;_=options.call.cancel.cancelled()=>return Err(cancelled()),_=tokio::time::sleep(options.poll_interval.mul_f64(factor))=>{}}
            }
        };
        tokio::time::timeout(options.timeout, wait)
            .await
            .map_err(|_| {
                let mut error = transport();
                error.message = "the chain did not finish within the wait timeout".into();
                error.details = Box::new(json!({"reason":"WAIT_TIMEOUT"}));
                error
            })?
    }
}
