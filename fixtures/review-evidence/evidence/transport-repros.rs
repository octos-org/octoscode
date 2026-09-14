fn outer_review_630_fixture(silent_hello: bool) -> (tempfile::TempDir, std::path::PathBuf, ProtocolAppUiBackend) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("caps.sh");
    let marker = dir.path().join("caps.log");
    std::fs::write(&script, r#"#!/bin/sh
marker="$1"
silent="$2"
n=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"method":"client_hello"'*)
      if [ "$silent" = no ]; then
        printf '{"jsonrpc":"2.0","id":"%s","error":{"code":-32601,"message":"method not found"}}\n' "$id"
      fi
      ;;
    *'"method":"config/capabilities/list"'*)
      n=$((n+1))
      if { [ "$silent" = no ] && [ "$n" -eq 1 ]; } || { [ "$silent" = yes ] && [ "$n" -eq 2 ]; }; then
        printf '{"jsonrpc":"2.0","id":"%s","result":{"capabilities":{"version":{"protocol":"octos-ui/v1alpha1","schema_version":1,"jsonrpc":"2.0"},"capabilities_schema_version":2,"supported_methods":["profile/llm/catalog"],"supported_notifications":[],"supported_features":[]}}}\n' "$id"
      fi
      printf 'CAPS\n' >> "$marker"
      ;;
  esac
done
"#).unwrap();
    let mut backend = ProtocolAppUiBackend::new(AppUiLaunch {
        endpoint: Some(AppUiEndpoint::stdio(format!("sh {} {} {}",script.display(),marker.display(),if silent_hello {"yes"} else {"no"}))),
        ..AppUiLaunch::default()
    });
    backend.bootstrap().unwrap();
    (dir, marker, backend)
}

#[test]
fn outer_review_630_no_first_frame_must_retry_after_grace() {
    let (_dir, _marker, mut backend) = outer_review_630_fixture(true);
    backend.client_hello_barrier.as_mut().unwrap().started_at = Instant::now() - STDIO_CHILD_STARTUP_GRACE - Duration::from_secs(1);
    backend.check_protocol_barrier_timeouts();
    assert!(backend.client_hello_barrier.is_none());
    assert!(backend.stdio_child_startup_pending());
    let probe = backend.capabilities_probe.as_mut().expect("first request released after hello grace");
    assert_eq!(probe.attempts,1);
    probe.sent_at = Instant::now() - CAPABILITIES_RESPONSE_TIMEOUT - Duration::from_secs(1);
    backend.check_protocol_barrier_timeouts();
    assert_eq!(backend.capabilities_probe.as_ref().unwrap().attempts,2,"Expired startup grace must allow the second capabilities request without needing an unrelated first frame");
}

#[test]
fn outer_review_630_queued_success_must_survive_retry_deadline() {
    let (_dir, marker, mut backend) = outer_review_630_fixture(false);
    let deadline = Instant::now()+Duration::from_secs(3);
    while backend.capabilities_probe.is_none() && Instant::now()<deadline {
        backend.next_event().unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(backend.capabilities_probe.is_some());
    while !marker.exists() && Instant::now()<deadline { thread::sleep(Duration::from_millis(5)); }
    assert!(marker.exists(),"fixture answered first request");
    thread::sleep(Duration::from_millis(30));
    backend.capabilities_probe.as_mut().unwrap().sent_at = Instant::now()-CAPABILITIES_RESPONSE_TIMEOUT-Duration::from_secs(1);
    let until=Instant::now()+Duration::from_millis(500);
    let mut received=false;
    while Instant::now()<until {
        if matches!(backend.next_event().unwrap(),Some(ClientEvent::Capabilities(_))) {received=true;break;}
        thread::sleep(Duration::from_millis(5));
    }
    assert!(received,"A valid first response queued before polling was discarded when retry removed its request id");
}
