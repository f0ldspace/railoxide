//! Progressive identity verification through the real actor and default executor.
//!
//! Every wait is bounded in real time so a verification barrier fails quickly instead of hanging.

use super::*;

const BOUND: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(5);
/// Time for the broker to start a further probe if it would exceed the probe budget.
const SETTLE: Duration = Duration::from_millis(200);
/// Production identity-probe budget: one of the broker's HTTP slots stays available to reads.
const PROBE_BUDGET: usize = DEFAULT_MAX_IN_FLIGHT - 1;
const IDENTITY_METHOD: &str = "eth_chainId";
const LONG_TIMEOUT: Duration = Duration::from_secs(30);

type Submitted =
    tokio::task::JoinHandle<Result<Vec<Result<RpcResult, RpcBrokerError>>, RpcBrokerError>>;

/// Opens once and stays open, releasing current and later waiters.
#[derive(Clone)]
struct Latch(Arc<Semaphore>);

impl Latch {
    fn new() -> Self {
        Self(Arc::new(Semaphore::new(0)))
    }

    fn open(&self) {
        self.0.close();
    }

    fn is_open(&self) -> bool {
        self.0.is_closed()
    }

    async fn wait(&self) {
        // The semaphore never holds permits, so acquisition returns only once it is closed.
        let _ = self.0.acquire().await;
    }
}

/// Concurrent physical requests across every mock that shares this value.
#[derive(Clone, Default)]
struct Traffic {
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
    active_identity: Arc<AtomicUsize>,
    maximum_identity: Arc<AtomicUsize>,
}

impl Traffic {
    fn track(&self, identity: bool) -> InFlight {
        let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(current, Ordering::SeqCst);
        if identity {
            let current = self.active_identity.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum_identity.fetch_max(current, Ordering::SeqCst);
        }
        InFlight {
            traffic: self.clone(),
            identity,
        }
    }

    fn identity_in_flight(&self) -> usize {
        self.active_identity.load(Ordering::SeqCst)
    }

    fn assert_within_production_bounds(&self) {
        let identity = self.maximum_identity.load(Ordering::SeqCst);
        let total = self.maximum.load(Ordering::SeqCst);
        assert!(
            identity <= PROBE_BUDGET,
            "{identity} concurrent identity requests exceed the probe budget of {PROBE_BUDGET}"
        );
        assert!(
            total <= DEFAULT_MAX_IN_FLIGHT,
            "{total} concurrent physical requests exceed {DEFAULT_MAX_IN_FLIGHT}"
        );
    }
}

struct InFlight {
    traffic: Traffic,
    identity: bool,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.traffic.active.fetch_sub(1, Ordering::SeqCst);
        if self.identity {
            self.traffic.active_identity.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct EndpointState {
    responder: RpcResponder,
    identity_gate: Option<Latch>,
    read_received: Latch,
    methods: Arc<Mutex<Vec<String>>>,
    traffic: Traffic,
}

struct MockEndpoint {
    url: Url,
    server: tokio::task::JoinHandle<()>,
    /// Opens when the first requested read, rather than an identity check, arrives.
    read_received: Latch,
    methods: Arc<Mutex<Vec<String>>>,
}

impl MockEndpoint {
    fn reads(&self) -> Vec<String> {
        self.methods
            .lock()
            .expect("method log lock")
            .iter()
            .filter(|method| method.as_str() != IDENTITY_METHOD)
            .cloned()
            .collect()
    }
}

/// Mock RPC endpoint whose identity responses wait for `identity_gate` when one is given.
async fn spawn_endpoint(
    responder: RpcResponder,
    identity_gate: Option<Latch>,
    traffic: &Traffic,
) -> MockEndpoint {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind RPC mock");
    let address = listener.local_addr().expect("RPC mock address");
    let read_received = Latch::new();
    let methods = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(EndpointState {
        responder,
        identity_gate,
        read_received: read_received.clone(),
        methods: Arc::clone(&methods),
        traffic: traffic.clone(),
    });
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_endpoint(stream, Arc::clone(&state)));
        }
    });
    MockEndpoint {
        url: Url::parse(&format!("http://{address}")).expect("RPC mock URL"),
        server,
        read_received,
        methods,
    }
}

async fn serve_endpoint(mut stream: TcpStream, state: Arc<EndpointState>) -> std::io::Result<()> {
    let Some(request) = read_json_request(&mut stream).await? else {
        return Ok(());
    };
    let method = request["method"].as_str().unwrap_or_default().to_owned();
    let identity = method == IDENTITY_METHOD;
    state.methods.lock().expect("method log lock").push(method);
    let in_flight = state.traffic.track(identity);
    if identity {
        if let Some(gate) = &state.identity_gate {
            // A client that abandons a stalled check closes its connection; stop counting it.
            let mut probe = [0_u8; 1];
            tokio::select! {
                () = gate.wait() => {}
                _ = stream.read(&mut probe) => return Ok(()),
            }
        }
    } else {
        state.read_received.open();
    }
    let response = serde_json::to_vec(&(state.responder)(request)).expect("RPC mock JSON response");
    drop(in_flight);
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(&response).await
}

async fn read_json_request(stream: &mut TcpStream) -> std::io::Result<Option<Value>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let body_start = header_end + 4;
            let body_end = body_start + content_length(&request[..body_start]);
            if request.len() >= body_end {
                return Ok(Some(
                    serde_json::from_slice(&request[body_start..body_end])
                        .expect("RPC mock JSON request"),
                ));
            }
        }
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Ok(None);
        }
        request.extend_from_slice(&buffer[..read]);
    }
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn member_results() -> [Bytes; 2] {
    [Bytes::from_static(b"\x01"), Bytes::from_static(b"\x02")]
}

/// Verifies chain 1 and serves every read: aggregates with `member_results`, others with `0x42`.
fn healthy_responder() -> RpcResponder {
    Arc::new(|request: Value| match request["method"].as_str() {
        Some(IDENTITY_METHOD) => rpc_result(&request, &json!("0x1")),
        Some("eth_call") => aggregate_response(
            &request,
            member_results()
                .into_iter()
                .map(|result| (true, result))
                .collect(),
        ),
        _ => rpc_result(&request, &json!("0x42")),
    })
}

/// Verifies chain 1 but fails every read with an endpoint error that remains failover eligible.
fn failing_read_responder() -> RpcResponder {
    Arc::new(|request: Value| {
        if request["method"] == IDENTITY_METHOD {
            rpc_result(&request, &json!("0x1"))
        } else {
            rpc_error(&request, -32603)
        }
    })
}

#[derive(Clone, Copy)]
enum Shape {
    Individual,
    Aggregate,
}

impl Shape {
    fn route(self, endpoints: Vec<Url>) -> RpcRoute {
        let chain = RpcChainRoute::new(1, endpoints).with_identity_verification();
        RpcRoute::from(match self {
            Self::Individual => chain,
            Self::Aggregate => chain.with_multicall(Address::from([0x4d_u8; 20])),
        })
    }

    fn reads(self, seed: u8) -> Vec<RpcRead> {
        match self {
            Self::Individual => vec![method_read("eth_gasPrice")],
            Self::Aggregate => (1..=2)
                .map(|offset| {
                    RpcRead::eth_call(
                        Address::from([seed + offset; 20]),
                        Bytes::from_static(b"\x70\xa0\x82\x31"),
                    )
                })
                .collect(),
        }
    }

    fn expected(self) -> Vec<Result<RpcResult, RpcBrokerError>> {
        match self {
            Self::Individual => vec![Ok(RpcResult::new(json!("0x42")))],
            Self::Aggregate => member_results()
                .into_iter()
                .map(data_result)
                .map(Ok)
                .collect(),
        }
    }

    const fn method(self) -> &'static str {
        match self {
            Self::Individual => "eth_gasPrice",
            Self::Aggregate => "eth_call",
        }
    }
}

fn method_read(method: &str) -> RpcRead {
    RpcRead::from_method_params(method, json!([]), 1).expect("allowlisted read")
}

fn long_lived(route: RpcRoute) -> RpcRoute {
    route
        .with_request_timeout(LONG_TIMEOUT)
        .with_attempt_timeout(LONG_TIMEOUT)
}

/// Caches a successful identity check on `route` and its clones without involving the broker.
async fn warm_identity(route: &RpcRoute, url: &Url) {
    let endpoint = route
        .endpoints()
        .iter()
        .find(|endpoint| endpoint.expose_url() == url)
        .expect("endpoint belongs to the route");
    route
        .chain_route()
        .verify_endpoint_identity(&reqwest::Client::new(), endpoint, BOUND)
        .await
        .expect("warm endpoint identity");
}

async fn submit_within(
    broker: &RpcBroker,
    route: &RpcRoute,
    reads: Vec<RpcRead>,
    what: &str,
) -> Vec<Result<RpcResult, RpcBrokerError>> {
    time::timeout(
        BOUND,
        broker.submit(RpcSubmission::new(route.clone(), reads, test_origin())),
    )
    .await
    .unwrap_or_else(|_| panic!("{what} did not complete within {BOUND:?}"))
    .expect("broker admits the submission")
}

fn spawn_submit(broker: &Arc<RpcBroker>, route: RpcRoute, reads: Vec<RpcRead>) -> Submitted {
    let broker = Arc::clone(broker);
    tokio::spawn(async move {
        broker
            .submit(RpcSubmission::new(route, reads, test_origin()))
            .await
    })
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    time::timeout(BOUND, async {
        while !condition() {
            time::sleep(POLL).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
}

/// Submits a read on a fresh single-endpoint route whose identity check stalls on `gate`.
async fn stalled_verification_job(
    broker: &Arc<RpcBroker>,
    traffic: &Traffic,
    gate: &Latch,
) -> (MockEndpoint, Submitted) {
    let endpoint = spawn_endpoint(healthy_responder(), Some(gate.clone()), traffic).await;
    let route = long_lived(Shape::Individual.route(vec![endpoint.url.clone()]));
    let task = spawn_submit(broker, route, Shape::Individual.reads(0));
    (endpoint, task)
}

async fn release_stalled_jobs(gate: &Latch, jobs: Vec<(MockEndpoint, Submitted)>) {
    gate.open();
    for (endpoint, task) in jobs {
        let results = time::timeout(BOUND, task)
            .await
            .expect("released verification job completes")
            .expect("submission task")
            .expect("broker admits the submission");
        assert_eq!(results, Shape::Individual.expected());
        endpoint.server.abort();
    }
}

/// Warms a single-endpoint route through the broker so later reads on it are cached-ready.
async fn warmed_ready_route(broker: &RpcBroker, traffic: &Traffic) -> (MockEndpoint, RpcRoute) {
    let endpoint = spawn_endpoint(healthy_responder(), None, traffic).await;
    let route = Shape::Individual.route(vec![endpoint.url.clone()]);
    let results = submit_within(
        broker,
        &route,
        Shape::Individual.reads(0),
        "identity warm-up read",
    )
    .await;
    assert_eq!(results, Shape::Individual.expected());
    (endpoint, route)
}

#[tokio::test]
async fn individual_read_fails_over_to_endpoint_verified_after_first_attempt() {
    let traffic = Traffic::default();
    let ready = spawn_endpoint(failing_read_responder(), None, &traffic).await;
    // The fallback verifies only after the ready endpoint has received the requested read, so a
    // job that verifies every endpoint before its first read never gets an answer. It is also
    // configured first, so the read must start on the cached endpoint later in configured order
    // while the earlier candidate's check is pending.
    let fallback = spawn_endpoint(
        healthy_responder(),
        Some(ready.read_received.clone()),
        &traffic,
    )
    .await;
    let route = long_lived(Shape::Individual.route(vec![fallback.url.clone(), ready.url.clone()]));
    warm_identity(&route, &ready.url).await;
    let broker = test_broker(Duration::from_millis(1), DEFAULT_MAX_IN_FLIGHT);

    let results = submit_within(
        &broker,
        &route,
        Shape::Individual.reads(0),
        "fallback to the endpoint verified after the first attempt",
    )
    .await;

    assert_eq!(results, Shape::Individual.expected());
    assert_eq!(ready.reads(), [Shape::Individual.method()]);
    assert_eq!(fallback.reads(), [Shape::Individual.method()]);
    drop(broker);
    ready.server.abort();
    fallback.server.abort();
}

#[tokio::test]
async fn cached_ready_read_runs_while_four_jobs_await_identity_verification() {
    let traffic = Traffic::default();
    let broker = test_broker(Duration::from_millis(1), DEFAULT_MAX_IN_FLIGHT);
    let (ready, ready_route) = warmed_ready_route(&broker, &traffic).await;
    let stall = Latch::new();
    // Three of these jobs probe; the fourth waits for probe capacity.
    let mut stalled = Vec::new();
    for _ in 0..DEFAULT_MAX_IN_FLIGHT {
        stalled.push(stalled_verification_job(&broker, &traffic, &stall).await);
    }
    wait_until(
        "the probe budget is occupied by stalled identity checks",
        || traffic.identity_in_flight() >= PROBE_BUDGET,
    )
    .await;
    time::sleep(SETTLE).await;
    // A fifth verification-only job now heads the shared origin lane ahead of the ready read.
    stalled.push(stalled_verification_job(&broker, &traffic, &stall).await);
    time::sleep(SETTLE).await;

    let results = submit_within(
        &broker,
        &ready_route,
        vec![method_read("eth_blockNumber")],
        "cached-ready read behind stalled verification",
    )
    .await;

    assert_eq!(results, vec![Ok(RpcResult::new(json!("0x42")))]);
    assert_eq!(ready.reads(), ["eth_gasPrice", "eth_blockNumber"]);
    traffic.assert_within_production_bounds();
    release_stalled_jobs(&stall, stalled).await;
    drop(broker);
    ready.server.abort();
}

#[tokio::test]
async fn parked_aggregate_resumes_after_late_identity_while_ready_read_runs() {
    let traffic = Traffic::default();
    let broker = test_broker(Duration::from_millis(1), DEFAULT_MAX_IN_FLIGHT);
    let (ready, ready_route) = warmed_ready_route(&broker, &traffic).await;
    let stall = Latch::new();
    let failing = spawn_endpoint(failing_read_responder(), None, &traffic).await;
    let fallback = spawn_endpoint(healthy_responder(), Some(stall.clone()), &traffic).await;
    let parked_route =
        long_lived(Shape::Aggregate.route(vec![failing.url.clone(), fallback.url.clone()]));
    // Warm before the stalls so this direct check never overlaps broker-owned probes.
    warm_identity(&parked_route, &failing.url).await;
    let mut stalled = Vec::new();
    for _ in 0..PROBE_BUDGET {
        stalled.push(stalled_verification_job(&broker, &traffic, &stall).await);
    }
    wait_until(
        "the probe budget is occupied by stalled identity checks",
        || traffic.identity_in_flight() >= PROBE_BUDGET,
    )
    .await;

    // Its only verified endpoint fails, and the fallback's probe must wait for the budget.
    let parked = spawn_submit(&broker, parked_route, Shape::Aggregate.reads(0x20));
    wait_until("the verified endpoint receives the aggregate", || {
        failing.read_received.is_open()
    })
    .await;
    let results = submit_within(
        &broker,
        &ready_route,
        vec![method_read("eth_maxPriorityFeePerGas")],
        "cached-ready read while another job awaits fallback verification",
    )
    .await;
    assert_eq!(results, vec![Ok(RpcResult::new(json!("0x42")))]);
    assert!(!parked.is_finished());
    assert_eq!(ready.reads(), ["eth_gasPrice", "eth_maxPriorityFeePerGas"]);

    stall.open();
    let resumed = time::timeout(BOUND, parked)
        .await
        .expect("parked aggregate resumes after its fallback verifies")
        .expect("submission task")
        .expect("broker admits the submission");
    assert_eq!(resumed, Shape::Aggregate.expected());
    assert_eq!(failing.reads(), [Shape::Aggregate.method()]);
    assert_eq!(fallback.reads(), [Shape::Aggregate.method()]);
    traffic.assert_within_production_bounds();
    release_stalled_jobs(&stall, stalled).await;
    drop(broker);
    ready.server.abort();
    failing.server.abort();
    fallback.server.abort();
}

#[tokio::test]
async fn parked_jobs_wake_when_a_route_clone_verifies_an_endpoint() {
    let traffic = Traffic::default();
    let broker = test_broker(Duration::from_millis(1), DEFAULT_MAX_IN_FLIGHT);
    let stall = Latch::new();
    let failing = spawn_endpoint(failing_read_responder(), None, &traffic).await;
    let stalled_endpoint = spawn_endpoint(healthy_responder(), Some(stall.clone()), &traffic).await;
    let healthy = spawn_endpoint(healthy_responder(), None, &traffic).await;
    // Only the failing endpoint starts verified. A job needs a verified endpoint to start without
    // a runner; one with none waits in the queue instead of parking.
    let route = long_lived(Shape::Individual.route(vec![
        failing.url.clone(),
        stalled_endpoint.url.clone(),
        healthy.url.clone(),
    ]));
    warm_identity(&route, &failing.url).await;
    let mut stalled = Vec::new();
    for _ in 0..PROBE_BUDGET {
        stalled.push(stalled_verification_job(&broker, &traffic, &stall).await);
    }
    wait_until(
        "the probe budget is occupied by stalled identity checks",
        || traffic.identity_in_flight() >= PROBE_BUDGET,
    )
    .await;

    // Takes the last probe runner; its probes wait for the occupied budget.
    let probing = spawn_submit(&broker, route.clone(), Shape::Individual.reads(0));
    wait_until("the first job's read fails", || failing.reads().len() == 1).await;
    time::sleep(SETTLE).await;
    // Starts without a runner, so after its read fails it waits for one.
    let runnerless = spawn_submit(&broker, route.clone(), vec![method_read("eth_blockNumber")]);
    wait_until("the second job's read fails", || failing.reads().len() == 2).await;
    time::sleep(SETTLE).await;
    assert!(!probing.is_finished());
    assert!(!runnerless.is_finished());

    // Verified outside the broker, as sync admission does on its own clone of the route.
    warm_identity(&route, &healthy.url).await;

    for job in [probing, runnerless] {
        let results = time::timeout(BOUND, job)
            .await
            .expect("parked job resumes on the newly verified endpoint")
            .expect("submission task")
            .expect("broker admits the submission");
        assert_eq!(results, Shape::Individual.expected());
    }
    assert!(!stall.is_open());
    let mut served = healthy.reads();
    served.sort();
    assert_eq!(served, ["eth_blockNumber", "eth_gasPrice"]);
    assert!(stalled_endpoint.reads().is_empty());
    release_stalled_jobs(&stall, stalled).await;
    drop(broker);
    failing.server.abort();
    stalled_endpoint.server.abort();
    healthy.server.abort();
}

#[tokio::test]
async fn queued_reads_start_when_their_own_route_verifies_outside_the_broker() {
    let traffic = Traffic::default();
    let broker = test_broker(Duration::from_millis(1), DEFAULT_MAX_IN_FLIGHT);
    let stall = Latch::new();
    // Three of these jobs probe; the fourth waits for probe capacity. Every runner is held.
    let mut stalled = Vec::new();
    for _ in 0..DEFAULT_MAX_IN_FLIGHT {
        stalled.push(stalled_verification_job(&broker, &traffic, &stall).await);
    }
    wait_until(
        "the probe budget is occupied by stalled identity checks",
        || traffic.identity_in_flight() >= PROBE_BUDGET,
    )
    .await;
    time::sleep(SETTLE).await;

    // Equal routes built separately, as from two configurations, keep separate caches. Their
    // reads differ so the actor does not merge them under one key.
    let healthy = spawn_endpoint(healthy_responder(), None, &traffic).await;
    let first_route = long_lived(Shape::Individual.route(vec![healthy.url.clone()]));
    let second_route = long_lived(Shape::Individual.route(vec![healthy.url.clone()]));
    let first = spawn_submit(
        &broker,
        first_route.clone(),
        vec![method_read("eth_gasPrice")],
    );
    let second = spawn_submit(
        &broker,
        second_route.clone(),
        vec![method_read("eth_blockNumber")],
    );
    time::sleep(SETTLE).await;
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    assert!(healthy.reads().is_empty());

    // Verified outside the broker, as sync admission does, and only in the second cache.
    warm_identity(&second_route, &healthy.url).await;
    let results = time::timeout(BOUND, second)
        .await
        .expect("queued read starts once its route verifies")
        .expect("submission task")
        .expect("broker admits the submission");
    assert_eq!(results, vec![Ok(RpcResult::new(json!("0x42")))]);
    assert!(!stall.is_open());
    assert!(!first.is_finished());

    warm_identity(&first_route, &healthy.url).await;
    let results = time::timeout(BOUND, first)
        .await
        .expect("queued read starts once its route verifies")
        .expect("submission task")
        .expect("broker admits the submission");
    assert_eq!(results, vec![Ok(RpcResult::new(json!("0x42")))]);
    assert!(!stall.is_open());
    assert_eq!(healthy.reads(), ["eth_blockNumber", "eth_gasPrice"]);
    release_stalled_jobs(&stall, stalled).await;
    drop(broker);
    healthy.server.abort();
}

#[tokio::test]
async fn parked_read_keeps_its_dispatched_failure_when_the_caller_expires() {
    let traffic = Traffic::default();
    let stall = Latch::new();
    let failing = spawn_endpoint(failing_read_responder(), None, &traffic).await;
    let pending = spawn_endpoint(healthy_responder(), Some(stall.clone()), &traffic).await;
    let route = Shape::Individual.route(vec![failing.url.clone(), pending.url.clone()]);
    warm_identity(&route, &failing.url).await;
    // The actor is driven directly so the reply is observed after the caller deadline instead of
    // racing `RpcBroker::submit`'s own timeout.
    let (tx, rx) = mpsc::channel(1);
    let (_block_tx, block_rx) = mpsc::unbounded_channel();
    let actor = tokio::spawn(
        Actor::new(
            reqwest::Client::new(),
            rx,
            Duration::from_millis(1),
            DEFAULT_MAX_IN_FLIGHT,
            default_executor(),
            |_| {},
        )
        .run(block_rx),
    );
    let (reply, result) = oneshot::channel();
    let admission = Arc::new(Semaphore::new(1));
    tx.send(Command::Submit {
        submission: Box::new(RpcSubmission::new(
            route,
            Shape::Individual.reads(0),
            test_origin(),
        )),
        replies: vec![resolution::ReadReply::new(
            reply,
            Arc::new(admission.clone().acquire_owned().await.unwrap()),
        )],
        deadline: Some(Instant::now() + Duration::from_millis(300)),
    })
    .await
    .expect("actor accepts the submission");

    let result = time::timeout(BOUND, result)
        .await
        .expect("parked read settles at its deadline")
        .expect("actor replies");

    // The read reached the verified endpoint before parking for the stalled identity check, so
    // expiry must report that failure rather than a read that never dispatched.
    assert!(
        matches!(&result, Err(RpcBrokerError::Remote(remote)) if remote.code() == -32603),
        "unexpected parked result: {result:?}"
    );
    assert_eq!(failing.reads(), [Shape::Individual.method()]);
    assert!(pending.reads().is_empty());
    drop(tx);
    time::timeout(BOUND, actor)
        .await
        .expect("actor stops after channel closure")
        .expect("actor task");
    assert_eq!(admission.available_permits(), 1);
    stall.open();
    failing.server.abort();
    pending.server.abort();
}

#[tokio::test]
async fn single_slot_broker_verifies_and_reads_serially() {
    let traffic = Traffic::default();
    let first = spawn_endpoint(healthy_responder(), None, &traffic).await;
    let second = spawn_endpoint(healthy_responder(), None, &traffic).await;
    let route = Shape::Individual.route(vec![first.url.clone(), second.url.clone()]);
    let broker = test_broker(Duration::from_millis(1), 1);

    // Both jobs need verification, and one slot of each kind must still let them progress.
    let submissions = ["eth_gasPrice", "eth_blockNumber"]
        .map(|method| spawn_submit(&broker, route.clone(), vec![method_read(method)]));
    for submission in submissions {
        let results = time::timeout(BOUND, submission)
            .await
            .expect("single-slot submission completes")
            .expect("submission task")
            .expect("broker admits the submission");
        assert_eq!(results, vec![Ok(RpcResult::new(json!("0x42")))]);
    }

    assert_eq!(traffic.maximum.load(Ordering::SeqCst), 1);
    drop(broker);
    first.server.abort();
    second.server.abort();
}
