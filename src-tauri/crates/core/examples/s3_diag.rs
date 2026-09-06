//! Step-by-step S3 permission diagnostic. Credentials only via env vars:
//! OSYSTEMS_SYNC_S3_ACCESS_KEY, OSYSTEMS_SYNC_S3_SECRET_KEY, OSYSTEMS_SYNC_S3_BUCKET, OSYSTEMS_SYNC_S3_REGION.
//! Never prints secrets. `cargo run -p osystems-sync-core --example s3_diag`.

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use osystems_sync_core::config::{S3Config, StorageClass};
use osystems_sync_core::credentials::AwsCredentials;
use osystems_sync_core::throttle::Throttle;
use osystems_sync_core::uploaders::s3::{S3Options, S3Uploader};
use osystems_sync_core::uploaders::Uploader;

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("missing env {k}"))
}

fn show<T: std::fmt::Debug, E: std::fmt::Display>(step: &str, r: Result<T, E>) -> bool {
    match r {
        Ok(_) => {
            println!("OK   {step}");
            true
        }
        Err(e) => {
            let msg = e.to_string();
            println!(
                "FAIL {step} -> {}",
                msg.chars().take(220).collect::<String>()
            );
            false
        }
    }
}

#[tokio::main]
async fn main() {
    let ak = env("OSYSTEMS_SYNC_S3_ACCESS_KEY");
    let sk = env("OSYSTEMS_SYNC_S3_SECRET_KEY");
    let bucket = env("OSYSTEMS_SYNC_S3_BUCKET");
    let region = std::env::var("OSYSTEMS_SYNC_S3_REGION").unwrap_or_else(|_| "us-east-1".into());
    println!("bucket={bucket} region={region} key={}****", &ak[..4]);

    let http = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
        ))
        .build_https();
    let conf = aws_sdk_s3::config::Builder::new()
        .http_client(http)
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.clone()))
        .credentials_provider(Credentials::new(ak.clone(), sk.clone(), None, None, "diag"))
        .build();
    let c = aws_sdk_s3::Client::from_conf(conf);
    let probe = "exportacoes/.osystems-sync-probe";

    show(
        "head_bucket",
        c.head_bucket()
            .bucket(&bucket)
            .send()
            .await
            .map_err(|e| format!("{:?}", e.into_service_error())),
    );
    show(
        "list_objects_v2 (prefix exportacoes/)",
        c.list_objects_v2()
            .bucket(&bucket)
            .prefix("exportacoes/")
            .max_keys(1)
            .send()
            .await
            .map_err(|e| format!("{:?}", e.into_service_error())),
    );
    show(
        "put_object probe (0 B)",
        c.put_object()
            .bucket(&bucket)
            .key(probe)
            .body(Vec::<u8>::new().into())
            .send()
            .await
            .map_err(|e| format!("{:?}", e.into_service_error())),
    );
    show(
        "head_object probe",
        c.head_object()
            .bucket(&bucket)
            .key(probe)
            .send()
            .await
            .map_err(|e| format!("{:?}", e.into_service_error())),
    );
    show(
        "delete_object probe",
        c.delete_object()
            .bucket(&bucket)
            .key(probe)
            .send()
            .await
            .map_err(|e| format!("{:?}", e.into_service_error())),
    );
    let mp = c
        .create_multipart_upload()
        .bucket(&bucket)
        .key("exportacoes/.osystems-sync-mpu-probe")
        .send()
        .await;
    let ok = show(
        "create_multipart_upload",
        mp.as_ref().map(|_| ()).map_err(|e| format!("{:?}", e)),
    );
    if ok {
        let id = mp.unwrap().upload_id.unwrap_or_default();
        show(
            "list_parts",
            c.list_parts()
                .bucket(&bucket)
                .key("exportacoes/.osystems-sync-mpu-probe")
                .upload_id(&id)
                .send()
                .await
                .map_err(|e| format!("{:?}", e.into_service_error())),
        );
        show(
            "abort_multipart_upload",
            c.abort_multipart_upload()
                .bucket(&bucket)
                .key("exportacoes/.osystems-sync-mpu-probe")
                .upload_id(&id)
                .send()
                .await
                .map_err(|e| format!("{:?}", e.into_service_error())),
        );
    }

    let cfg = S3Config {
        enabled: true,
        region,
        bucket,
        prefix: "exportacoes/".into(),
        storage_class: StorageClass::Standard,
    };
    let up = S3Uploader::new(
        cfg,
        AwsCredentials {
            access_key_id: ak,
            secret_access_key: sk,
        },
        Throttle::new(0),
        S3Options::default(),
    )
    .await
    .expect("uploader");
    match up.test_connection().await {
        Ok(r) => println!(
            "APP test_connection -> ok={} {} ({} ms)",
            r.ok, r.message, r.latency_ms
        ),
        Err(e) => println!("APP test_connection -> ERR {e}"),
    }
    if std::env::var("OSYSTEMS_SYNC_S3_E2E").is_ok() {
        e2e(&up).await;
    }
}

/// Real end-to-end upload check (Fase 3 acceptance): 1 MiB simple put, 20 MiB
/// multipart, then the same 20 MiB again (must be skipped by the sha256 check).
/// Enabled with `OSYSTEMS_SYNC_S3_E2E=1`.
#[allow(dead_code)]
pub async fn e2e(up: &S3Uploader) {
    use osystems_sync_core::hash::sha256_file;
    use osystems_sync_core::uploaders::UploadRequest;
    use tokio_util::sync::CancellationToken;

    let dir = std::env::temp_dir().join("osync-s3-e2e");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    for (name, mib) in [
        ("e2e_small_1mib.bin", 1u64),
        ("e2e_multipart_20mib.bin", 20u64),
    ] {
        let path = dir.join(name);
        let mut data = vec![0u8; (mib * 1024 * 1024) as usize];
        let mut x: u32 = 0x9E37_79B9 ^ mib as u32;
        for b in data.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        std::fs::write(&path, &data).expect("write");
        let sha = sha256_file(path.clone()).await.expect("sha").to_string();
        for attempt in 1..=(if mib == 20 { 2 } else { 1 }) {
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let req = UploadRequest {
                local_path: path.clone(),
                remote_name: name.to_string(),
                size: data.len() as u64,
                sha256: sha.clone(),
                progress: tx,
                cancel: CancellationToken::new(),
                resume_state: None,
            };
            let started = std::time::Instant::now();
            let res = up.upload(req).await;
            let mut last = 0u64;
            while let Ok(p) = rx.try_recv() {
                last = p.sent;
            }
            match res {
                Ok(r) => println!(
                    "E2E {name} attempt {attempt}: OK remote_id={} skipped={} progress_last={last} in {} ms",
                    r.remote_id,
                    r.remote_state.as_ref().and_then(|v| v.get("skipped")).map(|v| v.to_string()).unwrap_or("false".into()),
                    started.elapsed().as_millis()
                ),
                Err(e) => println!("E2E {name} attempt {attempt}: FAIL {e}"),
            }
        }
    }
}
