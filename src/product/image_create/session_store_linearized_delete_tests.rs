//! session_store.rs「线性化删除」测试的外置模块：经 session_store.rs 内
//! `#[path = "session_store_linearized_delete_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod linearized_delete` 完全一致
//! （纯移动，无行为变化；满足 large_file_guard 的 1200 行上限，Task 20 线 A）。

use super::*;

#[tokio::test]
async fn tombstone_rejects_all_mutations_without_changing_record() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .expect("begin delete")
        .expect("lease");
    let tombstone = fixture.store.get(&session.id).await.unwrap().unwrap();
    assert_eq!(tombstone.session.status, SessionStatus::Deleting);
    assert_eq!(tombstone.generation, lease.token);

    assert!(matches!(
        fixture
            .store
            .append_message(
                &session.id,
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "late".to_string(),
                    ts: Utc::now(),
                },
            )
            .await,
        Err(ImageCreateError::SessionClosing)
    ));
    assert!(matches!(
        fixture
            .store
            .append_prompt_block(
                &session.id,
                PromptBlock {
                    content: "late".to_string(),
                    version: 0,
                    ts: None,
                },
            )
            .await,
        Err(ImageCreateError::SessionClosing)
    ));
    assert!(matches!(
        fixture
            .store
            .append_generation_result(
                &session.id,
                generation_result("late", "image/png", None, Some("aW1hZ2U=")),
            )
            .await,
        Err(ImageCreateError::SessionClosing)
    ));
    assert!(matches!(
        fixture
            .store
            .append_event(
                &session.id,
                SessionEvent {
                    kind: "late".to_string(),
                    message: "late".to_string(),
                    ts: Utc::now(),
                },
            )
            .await,
        Err(ImageCreateError::SessionClosing)
    ));
    assert!(matches!(
        fixture
            .store
            .update_session_meta(&session.id, Some("late".to_string()), None)
            .await,
        Err(ImageCreateError::SessionClosing)
    ));

    assert_eq!(
        fixture.store.get(&session.id).await.unwrap().unwrap(),
        tombstone
    );
}

#[tokio::test]
async fn concurrent_begin_delete_returns_exactly_one_lease() {
    let fixture = Fixture::new();
    let session = fixture.create().await;

    let (left, right) = tokio::join!(
        fixture.store.begin_delete(&session.id),
        fixture.store.begin_delete(&session.id)
    );
    let leases = [left.expect("left"), right.expect("right")];
    assert_eq!(leases.iter().filter(|lease| lease.is_some()).count(), 1);
}

#[tokio::test]
async fn finish_delete_removes_record_and_scratch_directory() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .unwrap()
        .unwrap();

    fixture
        .store
        .finish_delete(lease)
        .await
        .expect("finish delete");

    assert!(fixture.store.get(&session.id).await.unwrap().is_none());
    assert!(
        !fixture
            .paths
            .image_create_session_file(&session.id)
            .exists()
    );
    assert!(
        !fixture
            .paths
            .image_create_session_scratch_dir(&session.id)
            .exists()
    );
}

#[tokio::test]
async fn delete_session_removes_its_image_files() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let image_ids = [
        "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
        "1f2e3d4c-5b6a-7980-8997-b6c5d4e3f2a1",
    ];
    let media_types = ["image/png", "image/jpeg"];

    for ((image_id, media_type), contents) in image_ids
        .iter()
        .zip(media_types)
        .zip([b"first image".as_slice(), b"second image".as_slice()])
    {
        fixture
            .store
            .append_generation_result(
                &session.id,
                generation_result("migrated", media_type, Some((*image_id).to_string()), None),
            )
            .await
            .expect("append migrated result");
        write_image_atomic(&fixture.paths, image_id, media_type, contents)
            .await
            .expect("write image file");
    }

    let image_files: Vec<_> = image_ids
        .iter()
        .zip(media_types)
        .map(|(image_id, media_type)| {
            fixture
                .paths
                .image_create_image_file(image_id, media_type)
                .expect("image path")
        })
        .collect();
    assert!(image_files.iter().all(|path| path.exists()));

    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .expect("begin delete")
        .expect("lease");
    fixture
        .store
        .finish_delete(lease)
        .await
        .expect("finish delete");

    assert!(image_files.iter().all(|path| !path.exists()));
    assert!(
        !fixture
            .paths
            .image_create_session_file(&session.id)
            .exists()
    );
}

#[tokio::test]
async fn delete_session_continues_when_an_image_file_cannot_be_removed() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let image_id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    let media_type = "image/png";
    fixture
        .store
        .append_generation_result(
            &session.id,
            generation_result("migrated", media_type, Some(image_id.to_string()), None),
        )
        .await
        .expect("append migrated result");
    let image_path = fixture
        .paths
        .image_create_image_file(image_id, media_type)
        .expect("image path");
    tokio::fs::create_dir_all(&image_path)
        .await
        .expect("create non-removable image-file path");

    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .expect("begin delete")
        .expect("lease");
    fixture
        .store
        .finish_delete(lease)
        .await
        .expect("finish delete despite image cleanup failure");

    assert!(image_path.is_dir());
    assert!(
        !fixture
            .paths
            .image_create_session_file(&session.id)
            .exists()
    );
}

#[tokio::test]
async fn generation_token_detects_concurrent_record_modification() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .unwrap()
        .unwrap();
    let mut record = fixture.store.get(&session.id).await.unwrap().unwrap();
    record.generation += 1;
    fixture.store.write_record(&record).await.unwrap();

    assert!(matches!(
        fixture.store.finish_delete(lease).await,
        Err(ImageCreateError::Store(message)) if message.contains("concurrent modification")
    ));
    assert!(
        fixture
            .paths
            .image_create_session_file(&session.id)
            .exists()
    );
    assert!(
        fixture
            .paths
            .image_create_session_scratch_dir(&session.id)
            .exists()
    );
}

#[tokio::test]
async fn asynchronous_generation_completion_cannot_write_after_delete_begins() {
    let fixture = Fixture::new();
    let session = fixture.create().await;
    let lease = fixture
        .store
        .begin_delete(&session.id)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(
        fixture
            .store
            .append_generation_result(
                &session.id,
                generation_result("late completion", "image/png", None, Some("aW1hZ2U=")),
            )
            .await,
        Err(ImageCreateError::SessionClosing)
    ));
    fixture.store.finish_delete(lease).await.unwrap();
    assert!(fixture.store.get(&session.id).await.unwrap().is_none());
}
