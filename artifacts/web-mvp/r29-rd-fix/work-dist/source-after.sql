PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
INSERT INTO _sqlx_migrations VALUES(1,'core schema','2026-09-12 16:33:22',1,X'6b19818d97c46b377648d53878325d0be38168743f9cc0f98fa418f9cd235c053f40ec436eb15ad42b734e9871c2b671',7917875);
INSERT INTO _sqlx_migrations VALUES(2,'invariants','2026-09-12 16:33:22',1,X'457d54aed7ad3cb5a64b63a6549f1e1b99ecf66fe82e40b92beeb11305b360275b5a38f7d8980c9584ec75b8d96e754d',844542);
INSERT INTO _sqlx_migrations VALUES(3,'photos view unique','2026-09-12 16:33:22',1,X'8596e3982b6bd7692afd2ec5bd83110089ec2f2666e8aef00ab02438f762725a8bfbafa6f9e202fe8340cc31f6e56591',440208);
INSERT INTO _sqlx_migrations VALUES(4,'preparation pages','2026-09-12 16:33:22',1,X'0c85297346eafca9c957917330bc39ffcb19c0d490a3f459632a276ade2b99f4be7a46b01472e0f917dc8d6154c8ed59',2409584);
INSERT INTO _sqlx_migrations VALUES(5,'job execution','2026-09-12 16:33:22',1,X'175e59753dc525a366c95abb1d63c586d50f8f7ff1cf638935c3ea03167675468b074495761c51a349b146e109fa4345',3687833);
INSERT INTO _sqlx_migrations VALUES(6,'generation requests','2026-09-12 16:33:22',1,X'222520b11a01577b87341790a7c9f1db7d0182dde1f39fb950235dc8e2628ba26b65353485818907d36e4d3cbd365d26',1281500);
INSERT INTO _sqlx_migrations VALUES(7,'release manifest','2026-09-12 16:33:22',1,X'922524d19f69535d833a9fcdd98e3000fb696290c63b831f82084f737eaa5160acc32675680c23888a95d77d1dd0d8dc',4045333);
CREATE TABLE admins (
    id            TEXT PRIMARY KEY,
    -- 只存 Argon2 哈希；无默认值、无默认密码（contracts.md §2）。
    password_hash TEXT NOT NULL CHECK (length(password_hash) > 0),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
INSERT INTO admins VALUES('01a09677-6630-7469-a8f0-d0a5ed668921','$argon2id$v=19$m=19456,t=2,p=1$xUGDg2n54k4tdgDU8ogaUw$tN9B5oIMECHrlVv7O6NUDcSUSKW3bKbDSPH3czBtwoQ',1789230802480,1789230802480);
CREATE TABLE sessions (
    id                 TEXT PRIMARY KEY,
    admin_id           TEXT NOT NULL REFERENCES admins (id) ON DELETE CASCADE,
    -- 会话明文只返回 cookie，不落库/日志：这里只存哈希（contracts.md §2）。
    session_token_hash TEXT NOT NULL UNIQUE CHECK (length(session_token_hash) > 0),
    csrf_hash          TEXT NOT NULL CHECK (length(csrf_hash) > 0),
    created_at         INTEGER NOT NULL,
    expires_at         INTEGER NOT NULL,
    revoked_at         INTEGER,
    CHECK (expires_at > created_at)
);
CREATE TABLE items (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL CHECK (length(trim(name)) > 0),
    brand       TEXT,
    model       TEXT NOT NULL CHECK (length(trim(model)) > 0),
    variant     TEXT,
    revision    INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    archived_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
    -- 同品牌型号不强制唯一：允许“同型号不同配置”并存（contracts.md §2、REQ-010）。
);
INSERT INTO items VALUES('01a09677-6737-7665-a84b-335aaf2a4af8','备份测试物品',NULL,'X100V',NULL,1,NULL,1789230802743,1789230802743);
CREATE TABLE blobs (
    sha256        TEXT PRIMARY KEY
        CHECK (length(sha256) = 64 AND sha256 GLOB '[0-9a-f]*' AND NOT sha256 GLOB '*[^0-9a-f]*'),
    size          INTEGER NOT NULL CHECK (size >= 0),
    mime          TEXT NOT NULL,
    storage_state TEXT NOT NULL DEFAULT 'stored'
        CHECK (storage_state IN ('stored', 'quarantined', 'missing')),
    created_at    INTEGER NOT NULL
);
INSERT INTO blobs VALUES('e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda',1216,'application/pdf','stored',1789230802767);
INSERT INTO blobs VALUES('122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586',174,'image/jpeg','stored',1789230802783);
INSERT INTO blobs VALUES('972f6c241a5c0bfcec4af02a660218f9cd0689d77840fe5cf49a8ffe97edd010',39,'text/plain; charset=utf-8','stored',1789230802798);
INSERT INTO blobs VALUES('6040550076320303894e351b83bb7844d84ea5431ae7a209947026accd79c79b',39,'text/plain; charset=utf-8','stored',1789230802832);
INSERT INTO blobs VALUES('daa48f0d8a240072c9dea69d7fea52229e2c7ab37b5fea1126d02b594d329ccf',39,'text/plain; charset=utf-8','stored',1789230802866);
INSERT INTO blobs VALUES('0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890',12420,'image/png','stored',1789230802905);
INSERT INTO blobs VALUES('74b3edaa5bcc408629228edf8365bd691844025c8135c50cc4892bc1acdbb20b',1815,'application/json','stored',1789230802941);
INSERT INTO blobs VALUES('e694fab5444078cb515dd1ac67844a7a6599422116fde2f1e31e984bfc33ea69',2030,'application/json','stored',1789230802954);
INSERT INTO blobs VALUES('c017464ac0b990e826e3d2c527aeac4b351a169a09b10a4f84fb07959f19e4f1',1962,'application/json','stored',1789230802971);
INSERT INTO blobs VALUES('a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb',2912,'model/gltf-binary','stored',1789230803001);
INSERT INTO blobs VALUES('c2ec7def5b84a2bdad698be1bab9c14f0332255779d6a03c00a9c266b3d12465',5181,'application/json','stored',1789230803034);
CREATE TABLE documents (
    id              TEXT PRIMARY KEY,
    item_id         TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    source_asset_id TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    source_sha256   TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    title           TEXT NOT NULL,
    -- source_url 只作出处记录，服务端不据此发起抓取（contracts.md §2、REQ-012）。
    source_url      TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
INSERT INTO documents VALUES('01a09677-6750-74f5-a484-99c11a7355ec','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-674f-70f2-bb1c-9538edcf75b5','e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda','样例说明书',NULL,1789230802768,1789230802768);
CREATE TABLE preparations (
    id            TEXT PRIMARY KEY,
    document_id   TEXT NOT NULL REFERENCES documents (id) ON DELETE RESTRICT,
    source_sha256 TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    state         TEXT NOT NULL DEFAULT 'preparing'
        CHECK (state IN ('preparing', 'ready')),
    page_count    INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    revision      INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL, client_derived INTEGER NOT NULL DEFAULT 0
        CHECK (client_derived IN (0, 1)),
    -- ready 必须声明页数；“ready 后不可修改”由仓储层在事务内保证（T09）。
    CHECK (state <> 'ready' OR page_count IS NOT NULL)
);
INSERT INTO preparations VALUES('01a09677-6751-763c-ad1e-ebd69605a5a2','01a09677-6750-74f5-a484-99c11a7355ec','e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda','ready',3,5,1789230802769,1789230802869,1);
CREATE TABLE pages (
    preparation_id TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    page_number    INTEGER NOT NULL CHECK (page_number >= 1),
    text_asset_id  TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    image_asset_id TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL, viewport_json TEXT
        CHECK (viewport_json IS NULL OR json_valid(viewport_json)),
    PRIMARY KEY (preparation_id, page_number)
);
INSERT INTO pages VALUES('01a09677-6751-763c-ad1e-ebd69605a5a2',1,'01a09677-676e-734a-a55a-9f1d68a58820','01a09677-675f-7308-aa4f-9565dd03bf41',1789230802799,1789230802799,'{"width":1240,"height":1754,"rotation":0}');
INSERT INTO pages VALUES('01a09677-6751-763c-ad1e-ebd69605a5a2',2,'01a09677-6790-7502-9a37-55a2ef513671','01a09677-6781-760a-8978-ab3bcec49af9',1789230802833,1789230802833,'{"width":1240,"height":1754,"rotation":0}');
INSERT INTO pages VALUES('01a09677-6751-763c-ad1e-ebd69605a5a2',3,'01a09677-67b2-7077-8464-524fe3411e82','01a09677-67a2-7792-bd74-de0ffa60f833',1789230802867,1789230802867,'{"width":1240,"height":1754,"rotation":0}');
CREATE TABLE photos (
    id         TEXT PRIMARY KEY,
    item_id    TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    asset_id   TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    -- 视图方向以物品自身为参照（PRD A-04）；detail 不进入 Tripo 多视图请求。
    view       TEXT NOT NULL CHECK (view IN ('front', 'left', 'back', 'right', 'detail')),
    revision   INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
INSERT INTO photos VALUES('01a09677-67c7-76ed-9727-e33171282f19','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-67c6-7372-8101-7691cb5524b1','front',1,1789230802887,1789230802887);
INSERT INTO photos VALUES('01a09677-67da-7295-8842-86d4e36ef862','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-67d9-7383-b48c-7d3d654643c8','left',1,1789230802906,1789230802906);
CREATE TABLE generation_snapshots (
    id              TEXT PRIMARY KEY,
    item_id         TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    item_revision   INTEGER NOT NULL CHECK (item_revision >= 1),
    preparation_id  TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    photo_ids       TEXT NOT NULL CHECK (json_valid(photo_ids)),
    photo_hashes    TEXT NOT NULL CHECK (json_valid(photo_hashes)),
    provider_config TEXT NOT NULL CHECK (json_valid(provider_config)),
    prompt_version  TEXT NOT NULL,
    price_version   TEXT NOT NULL,
    budgets         TEXT NOT NULL CHECK (json_valid(budgets)),
    created_at      INTEGER NOT NULL
    -- 不保存 API key：provider_config 只允许非密钥配置（contracts.md §2/REQ-022）。
);
INSERT INTO generation_snapshots VALUES('01a09677-67e3-741e-826a-3d673234cbf8','01a09677-6737-7665-a84b-335aaf2a4af8',1,'01a09677-6751-763c-ad1e-ebd69605a5a2','["01a09677-67c7-76ed-9727-e33171282f19","01a09677-67da-7295-8842-86d4e36ef862"]','["122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586","0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890"]','{"manualAi":{"model":"gpt-5-mini","promptVersion":"manual_extract_v1"},"tripo":{"faceLimit":100000,"generateParts":false,"geometryQuality":"standard","model":"v3.1-20260211","pbr":true,"preset":"tripo-h-v3.1-standard","quad":false,"texture":true,"textureQuality":"standard"}}','manual_extract_v1','2026-09-11','{"authorized":{"manualAiUsdMicros":500000,"tripoCreditMinor":3000},"budgetNotice":"预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准","estimated":{"manualAiUsdMicros":4885,"tripoCreditMinor":3000},"priceVersion":"2026-09-11","quoteId":"01a09677-67dd-7146-91b1-da403091a711","upperBound":{"manualAiUsdMicros":8522,"tripoCreditMinor":3000}}',1789230802914);
CREATE TABLE jobs (
    id          TEXT PRIMARY KEY,
    item_id     TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    snapshot_id TEXT NOT NULL REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    status      TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'waiting_provider', 'retry_wait', 'needs_input',
        'submission_unknown', 'succeeded', 'failed', 'cancelled'
    )),
    revision    INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
INSERT INTO jobs VALUES('01a09677-67e3-741e-826a-3d6af9f52523','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-67e3-741e-826a-3d673234cbf8','succeeded',3,1789230802915,1789230962908);
CREATE TABLE job_stages (
    id              TEXT PRIMARY KEY,
    job_id          TEXT NOT NULL REFERENCES jobs (id) ON DELETE RESTRICT,
    stage_kind      TEXT NOT NULL CHECK (stage_kind IN (
        'freeze_inputs', 'manual_extract', 'manual_merge', 'tripo_upload',
        'tripo_submit', 'tripo_poll', 'model_download', 'model_validate',
        'assemble_draft'
    )),
    batch_index     INTEGER NOT NULL DEFAULT 0 CHECK (batch_index >= 0),
    page_set        TEXT CHECK (page_set IS NULL OR json_valid(page_set)),
    input_hash      TEXT NOT NULL,
    result_asset_id TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    usage_json      TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    status          TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'waiting_provider', 'retry_wait', 'needs_input',
        'submission_unknown', 'succeeded', 'failed', 'cancelled'
    )),
    lease_owner     TEXT,
    lease_epoch     INTEGER NOT NULL DEFAULT 0 CHECK (lease_epoch >= 0),
    lease_until     INTEGER,
    next_run_at     INTEGER,
    attempt_count   INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL, poll_count INTEGER NOT NULL DEFAULT 0
    CHECK (poll_count >= 0), last_error TEXT, needs_input_json TEXT
    CHECK (needs_input_json IS NULL OR json_valid(needs_input_json)),
    UNIQUE (job_id, stage_kind, batch_index),
    CHECK (stage_kind = 'manual_extract' OR batch_index = 0)
);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd0218c4c7e2','01a09677-67e3-741e-826a-3d6af9f52523','freeze_inputs',0,NULL,'498083ce028c250b3983e37a485ef509f884e5f3ac34de8afcb9914d5d985fbb',NULL,NULL,'succeeded',NULL,0,NULL,NULL,0,1789230802914,1789230802914,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd0366afde7d','01a09677-67e3-741e-826a-3d6af9f52523','manual_extract',0,'[1,2,3]','d84e6da1fd993c93182561107fcb8d7fe9380888b7f7cb9610a75c25a8a99c43','01a09677-680a-72da-938d-347ac82fe652','{"batchIndex":0,"diagnosticSha256":"74b3edaa5bcc408629228edf8365bd691844025c8135c50cc4892bc1acdbb20b","entityCounts":{"parts":2,"specs":1,"steps":1,"uncertainties":1},"errorCode":null,"errorSummary":null,"model":"gpt-5-mini","outcome":"completed","pages":[1,2,3],"priceVersion":"2026-09-11","producedKnowledge":true,"promptVersion":"manual_extract_v1","responseId":"resp_fixture_0001","schemaVersion":"manual_extract_v1","usage":{"inputTokens":1234,"outputTokens":321,"totalTokens":1555}}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230822908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd043a36c059','01a09677-67e3-741e-826a-3d6af9f52523','manual_merge',0,NULL,'548197bfc59d4a8e2efc311385707f82c1608037314aaf12fe5a47f96ba523da','01a09677-681b-715b-8eb8-f2108ecb3009','{"conflictCount":0,"coverage":{"batches":[{"batchIndex":0,"pages":[1,2,3],"partCount":2,"specCount":1,"stepCount":1,"uncertaintyCount":1}],"complete":true,"pageCount":3,"pages":[1,2,3]},"pageFrom":1,"pageTo":3,"partCount":2,"promptVersion":"manual_extract_v1","schemaVersion":"manual_extract_v1","specCount":1,"stepCount":1,"uncertaintyCount":1}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230842908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd05b28891a3','01a09677-67e3-741e-826a-3d6af9f52523','tripo_upload',0,NULL,'7b75880ceeee3656c91dd41b99bea717b3c1b6a3d82b96632522807c2bc355ca',NULL,'{"uploads":[{"sha256":"122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586","token":"token-0-t20","tokenField":"file_token","view":"front"},{"sha256":"0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890","token":"token-1-t20","tokenField":"file_token","view":"left"}]}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230862908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd0662f0428e','01a09677-67e3-741e-826a-3d6af9f52523','tripo_submit',0,NULL,'9e172a371bb31d1e58cb5b45b62f4cac5e705ee55951be80c75317194eb0c699',NULL,'{"endpoint":"/generation/multiview-to-model","remoteTaskId":"t20-fixture-task-0001","requestHash":"22f9eb7b8233d81205ad7168520de58507a180e6b60559e59a620dfe36a2e4bd"}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230882908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e4-7444-888b-cd07703199b5','01a09677-67e3-741e-826a-3d6af9f52523','tripo_poll',0,NULL,'0910a7938989f03efe58c084820aaaee72d28a2fe9ddf97018deef70bf496660',NULL,'{"billing":{"creditMinor":3000,"currency":"credit_minor","literal":"30","sourceField":"credits_consumed"},"dataKeys":["credits_consumed","output","progress","status","task_id"],"modelUrl":"http://127.0.0.1:60643/model.glb","normalizedStatus":"success","progress":"100","rawStatus":"success","remoteTaskId":"t20-fixture-task-0001","renderedImageUrl":"https://cdn.example.invalid/preview.png"}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230902908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e5-76f0-9c4a-0ea7d70f2d0b','01a09677-67e3-741e-826a-3d6af9f52523','model_download',0,NULL,'4f9501ee408623d0431bd7031238eb68d98c93a9ca470212a9fa28e16af25c50','01a09677-6839-73f4-af93-ec3acdea7db5','{"batchIndex": 0, "outcome": "refusal", "remoteTaskId": "task-rd29-usage", "billing": {"creditMinor": 3000}, "errorSummary": "模型拒答（refusal）：下载参考 https://cdn.rd29.invalid/p.png?sign=RD29-USAGE-CANARY-4e71 也失败"}','needs_input',NULL,1,NULL,NULL,0,1789230802914,1789230922908,0,'模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request for url (https://cdn.rd29.invalid/m.glb?sign=RD29-STAGE-CANARY-1d55&expires=9999999999)','[{"code": "download_insecure_scheme", "message": "模型下载必须使用 HTTPS（实际 http://cdn.rd29.invalid/m.glb?sign=RD29-NEEDS-CANARY-8c02）：拒绝下载"}, {"code": "retry", "message": "下载可安全重试（task-9）"}]');
INSERT INTO job_stages VALUES('01a09677-67e5-76f0-9c4a-0ea868724ef2','01a09677-67e3-741e-826a-3d6af9f52523','model_validate',0,NULL,'c2c3d662e706c12bb4d35a1b5f421927f2b510fc01219fb5a461ab95a3a3a638','01a09677-6839-73f4-af93-ec3acdea7db5','{"bounds":{"max":[1.0,1.0,1.0],"min":[-1.0,-1.0,-1.0],"triangles":12,"vertices":24},"images":1,"maxTextureDimension":16,"meshes":1,"modelRevisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","primitives":1,"sha256":"a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb","sizeBytes":2912,"triangles":12,"validation":"validated","vertices":24}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230942908,0,NULL,NULL);
INSERT INTO job_stages VALUES('01a09677-67e5-76f0-9c4a-0ea99be111f4','01a09677-67e3-741e-826a-3d6af9f52523','assemble_draft',0,NULL,'cd015e0573271ba2a4727229d3f5fadfe7360327f0ca78234d3c215615e56c68',NULL,'{"completeness":"complete","created":true,"draftId":"01a09677-683e-73c9-bd6a-6b0576f02b69","draftRevision":1,"missingCodes":[]}','succeeded',NULL,1,NULL,NULL,0,1789230802914,1789230962908,0,NULL,NULL);
CREATE TABLE provider_attempts (
    id             TEXT PRIMARY KEY,
    job_id         TEXT NOT NULL REFERENCES jobs (id) ON DELETE RESTRICT,
    stage_id       TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    request_hash   TEXT NOT NULL,
    submit_state   TEXT NOT NULL CHECK (submit_state IN (
        'intent', 'submitting', 'accepted', 'unknown', 'failed'
    )),
    remote_task_id TEXT,
    -- 同步 Manual AI 的 response_id 不等于可轮询任务（contracts.md §2）。
    response_id    TEXT,
    started_at     INTEGER NOT NULL,
    last_error     TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
INSERT INTO provider_attempts VALUES('01a09677-67eb-70d1-9d9e-1956c7e4c268','01a09677-67e3-741e-826a-3d6af9f52523','01a09677-67e4-7444-888b-cd0366afde7d','91631545fa0e7b5e88f006f764eb302de33c341e692c9ac50dd093042ad2a798','accepted',NULL,'resp_fixture_0001',1789230822908,'链接 https://cdn.rd29.invalid/x.glb?sign=RD29-ATTEMPT-CANARY-6f38 已过期，请重试',1789230822908,1789230822908);
INSERT INTO provider_attempts VALUES('01a09677-6823-70a3-bcfd-761e39d91685','01a09677-67e3-741e-826a-3d6af9f52523','01a09677-67e4-7444-888b-cd0662f0428e','22f9eb7b8233d81205ad7168520de58507a180e6b60559e59a620dfe36a2e4bd','accepted','t20-fixture-task-0001',NULL,1789230882908,'链接 https://cdn.rd29.invalid/x.glb?sign=RD29-ATTEMPT-CANARY-6f38 已过期，请重试',1789230882908,1789230882908);
CREATE TABLE idempotency_records (
    id              TEXT PRIMARY KEY,
    admin_id        TEXT NOT NULL REFERENCES admins (id) ON DELETE RESTRICT,
    method          TEXT NOT NULL,
    route           TEXT NOT NULL,
    "key"           TEXT NOT NULL,
    body_hash       TEXT NOT NULL,
    resource_id     TEXT,
    response_status INTEGER,
    created_at      INTEGER NOT NULL,
    UNIQUE (admin_id, method, route, "key")
);
INSERT INTO idempotency_records VALUES('01a09677-67e5-76f0-9c4a-0eaa9ab76b18','01a09677-6630-7469-a8f0-d0a5ed668921','POST','/api/v1/items/{id}/jobs','t20-rehearsal-publish-key','108fcd986a99b6a4b167f9ea7134462da13ec2db409309806db346b107d4cf2d','01a09677-67e3-741e-826a-3d6af9f52523',202,1789230802914);
INSERT INTO idempotency_records VALUES('01a09677-685a-7542-8e2f-0b886087ee17','01a09677-6630-7469-a8f0-d0a5ed668921','POST','/api/v1/items/{id}/drafts/{draftId}/publish','t20-rehearsal-publish-key','3b0460b422e33109cd5fd20f964e1699744b1a6b3c9b8b1bd95efa70c3fd3a51','01a09677-685a-7542-8e2f-0b866c8a1fbc',201,1789230803015);
CREATE TABLE cost_ledger (
    id            TEXT PRIMARY KEY,
    snapshot_id   TEXT NOT NULL REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    attempt_id    TEXT REFERENCES provider_attempts (id) ON DELETE RESTRICT,
    provider      TEXT NOT NULL CHECK (provider IN ('tripo', 'manual_ai')),
    currency      TEXT NOT NULL CHECK (currency IN ('credit_minor', 'usd_micros')),
    reserved      INTEGER NOT NULL CHECK (reserved >= 0),
    actual        INTEGER CHECK (actual IS NULL OR actual >= 0),
    state         TEXT NOT NULL CHECK (state IN ('reserved', 'settled', 'released', 'unknown')),
    price_version TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    CHECK (state <> 'settled' OR actual IS NOT NULL),
    CHECK (state <> 'unknown' OR actual IS NULL)
);
INSERT INTO cost_ledger VALUES('01a09677-67e3-741e-826a-3d6892de50fb','01a09677-67e3-741e-826a-3d673234cbf8',NULL,'tripo','credit_minor',3000,3000,'settled','2026-09-11',1789230802914,1789230902908);
INSERT INTO cost_ledger VALUES('01a09677-67e3-741e-826a-3d692afb93db','01a09677-67e3-741e-826a-3d673234cbf8',NULL,'manual_ai','usd_micros',8522,NULL,'reserved','2026-09-11',1789230802914,1789230802914);
CREATE TABLE model_revisions (
    id                  TEXT PRIMARY KEY,
    item_id             TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    asset_id            TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    sha256              TEXT NOT NULL
        CHECK (length(sha256) = 64 AND sha256 GLOB '[0-9a-f]*' AND NOT sha256 GLOB '*[^0-9a-f]*'),
    provider_attempt_id TEXT REFERENCES provider_attempts (id) ON DELETE RESTRICT,
    bounds              TEXT CHECK (bounds IS NULL OR json_valid(bounds)),
    validation_state    TEXT NOT NULL DEFAULT 'pending'
        CHECK (validation_state IN ('pending', 'validated', 'rejected')),
    created_at          INTEGER NOT NULL
);
INSERT INTO model_revisions VALUES('01a09677-683c-71cb-a57b-1ad0ccdc5166','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-6839-73f4-af93-ec3acdea7db5','a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb','01a09677-6823-70a3-bcfd-761e39d91685','{"max":[1.0,1.0,1.0],"min":[-1.0,-1.0,-1.0],"triangles":12,"vertices":24}','validated',1789230803004);
CREATE TABLE manual_drafts (
    id                TEXT PRIMARY KEY,
    item_id           TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    snapshot_id       TEXT NOT NULL UNIQUE REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    model_revision_id TEXT REFERENCES model_revisions (id) ON DELETE RESTRICT,
    revision          INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    status            TEXT NOT NULL DEFAULT 'needs_review'
        CHECK (status IN ('needs_review', 'ready')),
    knowledge_json    TEXT NOT NULL CHECK (json_valid(knowledge_json)),
    review_json       TEXT CHECK (review_json IS NULL OR json_valid(review_json)),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
INSERT INTO manual_drafts VALUES('01a09677-683e-73c9-bd6a-6b0576f02b69','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-67e3-741e-826a-3d673234cbf8','01a09677-683c-71cb-a57b-1ad0ccdc5166',5,'needs_review','{"schemaVersion":"manual_draft_v1","sourceJobId":"01a09677-67e3-741e-826a-3d6af9f52523","completeness":"complete","model":{"revisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","sha256":"a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb","validationState":"validated","assetId":"01a09677-6839-73f4-af93-ec3acdea7db5","bounds":{"max":[1.0,1.0,1.0],"min":[-1.0,-1.0,-1.0],"triangles":12,"vertices":24}},"knowledge":{"schemaVersion":"manual_extract_v1","promptVersion":"manual_extract_v1","pageFrom":1,"pageTo":3,"coverage":{"complete":true,"pageCount":3,"pages":[1,2,3],"batches":[{"batchIndex":0,"pages":[1,2,3],"partCount":2,"stepCount":1,"specCount":1,"uncertaintyCount":1}]},"parts":[{"id":"part-9e887e7c5e6d","name":"后盖","description":"机身背部可拆盖板，由四颗不脱落螺钉固定。","evidence":[{"documentId":"01a09677-6750-74f5-a484-99c11a7355ec","preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","pageNumber":1,"quote":"Loosen the four captive screws on the rear cover.","bbox":null,"derived":false}],"reviewStatus":"needs_review","sourceBatches":[0]},{"id":"part-62719753b9e5","name":"电池仓","description":"后盖内侧的电池安放区域。","evidence":[{"documentId":"01a09677-6750-74f5-a484-99c11a7355ec","preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","pageNumber":2,"quote":null,"bbox":null,"derived":false}],"reviewStatus":"needs_review","sourceBatches":[0]}],"steps":[{"id":"step-086dd7e64a3a","title":"取下后盖","orderedActions":["松开四颗不脱落螺钉","沿边缘取下后盖"],"partIds":["part-9e887e7c5e6d"],"evidence":[{"documentId":"01a09677-6750-74f5-a484-99c11a7355ec","preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","pageNumber":1,"quote":"Loosen the four captive screws on the rear cover.","bbox":null,"derived":false}],"safetyNotes":["操作前断开电源。"],"reviewStatus":"needs_review","sourceBatches":[0]}],"specs":[{"id":"spec-bf9abd54cec5","label":"供电","value":"DC 12 V / 2.5 A","evidence":[{"documentId":"01a09677-6750-74f5-a484-99c11a7355ec","preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","pageNumber":2,"quote":null,"bbox":null,"derived":false}],"reviewStatus":"needs_review","sourceBatches":[0]}],"uncertainties":[{"id":"uncertainty-7b39961cd275","topic":"螺钉扭矩","detail":"资料未给出拧紧扭矩数值。","pageNumbers":[],"sourceBatches":[0]}],"conflicts":[]},"hotspots":[{"id":"01a09677-6844-774c-be3b-287e44bb31af","partId":"part-9e887e7c5e6d","status":"confirmed","anchor":{"modelRevisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","modelSha256":"a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb","positionLocal":[0.3,0.1,0.2]}},{"id":"01a09677-6844-774c-be3b-287f95928447","partId":"part-62719753b9e5","status":"confirmed","anchor":{"modelRevisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","modelSha256":"a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb","positionLocal":[0.3,0.1,0.2]}}],"stepPoses":{},"missing":[]}','{"entities":{"part-62719753b9e5":{"reviewStatus":"confirmed","textOnly":false,"editedAt":1789230803009,"editedBy":"01a09677-6630-7469-a8f0-d0a5ed668921"},"part-9e887e7c5e6d":{"reviewStatus":"confirmed","textOnly":false,"editedAt":1789230803009,"editedBy":"01a09677-6630-7469-a8f0-d0a5ed668921"},"spec-bf9abd54cec5":{"reviewStatus":"confirmed","textOnly":false,"editedAt":1789230803009,"editedBy":"01a09677-6630-7469-a8f0-d0a5ed668921"},"step-086dd7e64a3a":{"reviewStatus":"confirmed","textOnly":false,"editedAt":1789230803009,"editedBy":"01a09677-6630-7469-a8f0-d0a5ed668921"}},"modelReview":{"modelRevisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","modelSha256":"a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb","loaded":true,"userConfirmed":true,"checkedAt":1789230803013,"loadedAt":1789230803013,"userConfirmedAt":1789230803013}}',1789230962908,1789230803015);
CREATE TABLE manual_releases (
    id                TEXT PRIMARY KEY,
    item_id           TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    draft_id          TEXT NOT NULL REFERENCES manual_drafts (id) ON DELETE RESTRICT,
    draft_revision    INTEGER NOT NULL CHECK (draft_revision >= 1),
    model_revision_id TEXT NOT NULL REFERENCES model_revisions (id) ON DELETE RESTRICT,
    manifest_asset_id TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    created_at        INTEGER NOT NULL
);
INSERT INTO manual_releases VALUES('01a09677-685a-7542-8e2f-0b866c8a1fbc','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-683e-73c9-bd6a-6b0576f02b69',4,'01a09677-683c-71cb-a57b-1ad0ccdc5166','01a09677-685a-7542-8e2f-0b852e27cfc5',1789230803015);
CREATE TABLE audit_events (
    id            TEXT PRIMARY KEY,
    entity_type   TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    -- actor 为管理员 id，或 'system'（服务器动作）。
    actor         TEXT,
    action        TEXT NOT NULL,
    result        TEXT NOT NULL,
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    created_at    INTEGER NOT NULL
);
INSERT INTO audit_events VALUES('01a09677-67e0-76a1-a194-af1c9108d1a2','quote','01a09677-67dd-7146-91b1-da403091a711','01a09677-6630-7469-a8f0-d0a5ed668921','generation_send_scope_confirmed','accepted','{"imagePages":0,"itemId":"01a09677-6737-7665-a84b-335aaf2a4af8","itemModel":"X100V","itemName":"备份测试物品","manualAiModel":"gpt-5-mini","pageFrom":1,"pageTo":3,"preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","priceVersion":"2026-09-11","promptVersion":"manual_extract_v1","quoteId":"01a09677-67dd-7146-91b1-da403091a711","textPages":3,"tripoModel":"v3.1-20260211","tripoParameters":{"faceLimit":100000,"generateParts":false,"geometryQuality":"standard","model":"v3.1-20260211","pbr":true,"preset":"tripo-h-v3.1-standard","quad":false,"texture":true,"textureQuality":"standard"},"tripoPreset":"tripo-h-v3.1-standard","tripoViews":[{"photoId":"01a09677-67c7-76ed-9727-e33171282f19","view":"front"},{"photoId":"01a09677-67da-7295-8842-86d4e36ef862","view":"left"}],"upperBound":{"manualAiUsdMicros":8522,"tripoCreditMinor":3000}}',1789230802912);
INSERT INTO audit_events VALUES('01a09677-67e5-76f0-9c4a-0eabc55e920a','job','01a09677-67e3-741e-826a-3d6af9f52523','01a09677-6630-7469-a8f0-d0a5ed668921','generation_job_created','accepted','{"authorized":{"manualAiUsdMicros":500000,"tripoCreditMinor":3000},"itemId":"01a09677-6737-7665-a84b-335aaf2a4af8","priceVersion":"2026-09-11","quoteId":"01a09677-67dd-7146-91b1-da403091a711","snapshotId":"01a09677-67e3-741e-826a-3d673234cbf8","upperBound":{"manualAiUsdMicros":8522,"tripoCreditMinor":3000}}',1789230802914);
INSERT INTO audit_events VALUES('01a09677-683f-7092-95d0-2dc9b33ea770','manual_draft','01a09677-683e-73c9-bd6a-6b0576f02b69','system','draft_assembled','created','{"completeness":"complete","jobId":"01a09677-67e3-741e-826a-3d6af9f52523","missingCount":0,"revision":1,"snapshotId":"01a09677-67e3-741e-826a-3d673234cbf8"}',1789230962908);
INSERT INTO audit_events VALUES('01a09677-6842-75f7-bef4-19725e75def3','manual_draft','01a09677-683e-73c9-bd6a-6b0576f02b69','01a09677-6630-7469-a8f0-d0a5ed668921','draft_review_updated','review','{"hotspotCount":0,"modelReviewConfirmed":false,"modelReviewLoaded":false,"reviewedEntityCount":4,"revision":2,"sections":["review"],"staleHotspotCount":0,"stepPoseCount":0}',1789230803009);
INSERT INTO audit_events VALUES('01a09677-6844-774c-be3b-2880aa314328','manual_draft','01a09677-683e-73c9-bd6a-6b0576f02b69','01a09677-6630-7469-a8f0-d0a5ed668921','draft_review_updated','knowledge','{"hotspotCount":2,"modelReviewConfirmed":false,"modelReviewLoaded":false,"reviewedEntityCount":4,"revision":3,"sections":["knowledge"],"staleHotspotCount":0,"stepPoseCount":0}',1789230803011);
INSERT INTO audit_events VALUES('01a09677-6846-71da-be57-4a19232e69dc','manual_draft','01a09677-683e-73c9-bd6a-6b0576f02b69','01a09677-6630-7469-a8f0-d0a5ed668921','draft_review_updated','review','{"hotspotCount":2,"modelReviewConfirmed":true,"modelReviewLoaded":true,"reviewedEntityCount":4,"revision":4,"sections":["review"],"staleHotspotCount":0,"stepPoseCount":0}',1789230803013);
INSERT INTO audit_events VALUES('01a09677-685a-7542-8e2f-0b87f528af3c','manual_release','01a09677-685a-7542-8e2f-0b866c8a1fbc','01a09677-6630-7469-a8f0-d0a5ed668921','release_published','published','{"draftId":"01a09677-683e-73c9-bd6a-6b0576f02b69","draftRevision":4,"draftRevisionAfterPublish":5,"hotspotCount":2,"itemId":"01a09677-6737-7665-a84b-335aaf2a4af8","manifestAssetId":"01a09677-685a-7542-8e2f-0b852e27cfc5","manifestSha256":"c2ec7def5b84a2bdad698be1bab9c14f0332255779d6a03c00a9c266b3d12465","modelRevisionId":"01a09677-683c-71cb-a57b-1ad0ccdc5166","textOnlyPartCount":0}',1789230803015);
CREATE TABLE job_stage_deps (
    stage_id            TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    depends_on_stage_id TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    created_at          INTEGER NOT NULL,
    PRIMARY KEY (stage_id, depends_on_stage_id),
    -- 自环不是合法依赖（DAG 无环；跨阶段成环由建单方保证，见 T11/T15 交接）。
    CHECK (stage_id <> depends_on_stage_id)
);
INSERT INTO job_stage_deps VALUES('01a09677-67e4-7444-888b-cd0366afde7d','01a09677-67e4-7444-888b-cd0218c4c7e2',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e4-7444-888b-cd043a36c059','01a09677-67e4-7444-888b-cd0366afde7d',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e4-7444-888b-cd05b28891a3','01a09677-67e4-7444-888b-cd0218c4c7e2',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e4-7444-888b-cd0662f0428e','01a09677-67e4-7444-888b-cd05b28891a3',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e4-7444-888b-cd07703199b5','01a09677-67e4-7444-888b-cd0662f0428e',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e5-76f0-9c4a-0ea7d70f2d0b','01a09677-67e4-7444-888b-cd07703199b5',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e5-76f0-9c4a-0ea868724ef2','01a09677-67e5-76f0-9c4a-0ea7d70f2d0b',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e5-76f0-9c4a-0ea99be111f4','01a09677-67e4-7444-888b-cd043a36c059',1789230802914);
INSERT INTO job_stage_deps VALUES('01a09677-67e5-76f0-9c4a-0ea99be111f4','01a09677-67e5-76f0-9c4a-0ea868724ef2',1789230802914);
CREATE TABLE quotes (
    id                  TEXT PRIMARY KEY,
    item_id             TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    preparation_id      TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    -- 多视图照片的冻结引用（photoId 与内容 sha256 一一对应；T07 QA 前置约束：
    -- photos 行可变，只存 id 会漏掉"同 id 换资产"）。
    photo_ids           TEXT NOT NULL CHECK (json_valid(photo_ids)),
    photo_hashes        TEXT NOT NULL CHECK (json_valid(photo_hashes)),
    input_hash          TEXT NOT NULL CHECK (length(input_hash) = 64),
    model_preset        TEXT NOT NULL,
    -- 非密钥的供应商配置快照（模型名/质量参数/prompt 版本）；不含 API key。
    provider_config     TEXT NOT NULL CHECK (json_valid(provider_config)),
    price_version       TEXT NOT NULL,
    price_snapshot_date TEXT NOT NULL,
    page_count          INTEGER NOT NULL CHECK (page_count >= 1),
    -- 最大输出 token（说明书 AI 的预算口径之一；批次数 × 每批上限）。
    max_output_tokens   INTEGER NOT NULL CHECK (max_output_tokens >= 0),
    -- 完整报价载荷（分列金额、保守上界、发送范围）；服务端回读时不依赖前端。
    quote_json          TEXT NOT NULL CHECK (json_valid(quote_json)),
    expires_at          INTEGER NOT NULL,
    confirmed_at        INTEGER,
    confirmation_json   TEXT CHECK (confirmation_json IS NULL OR json_valid(confirmation_json)),
    consumed_at         INTEGER,
    consumed_job_id     TEXT REFERENCES jobs (id) ON DELETE RESTRICT,
    created_at          INTEGER NOT NULL,
    CHECK (consumed_at IS NULL OR consumed_job_id IS NOT NULL),
    CHECK (confirmed_at IS NULL OR confirmation_json IS NOT NULL)
);
INSERT INTO quotes VALUES('01a09677-67dd-7146-91b1-da403091a711','01a09677-6737-7665-a84b-335aaf2a4af8','01a09677-6751-763c-ad1e-ebd69605a5a2','["01a09677-67c7-76ed-9727-e33171282f19","01a09677-67da-7295-8842-86d4e36ef862"]','["122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586","0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890"]','498083ce028c250b3983e37a485ef509f884e5f3ac34de8afcb9914d5d985fbb','tripo-h-v3.1-standard','{"manualAi":{"model":"gpt-5-mini","promptVersion":"manual_extract_v1"},"tripo":{"faceLimit":100000,"generateParts":false,"geometryQuality":"standard","model":"v3.1-20260211","pbr":true,"preset":"tripo-h-v3.1-standard","quad":false,"texture":true,"textureQuality":"standard"}}','2026-09-11','2026-09-11',3,4096,'{"id":"01a09677-67dd-7146-91b1-da403091a711","itemId":"01a09677-6737-7665-a84b-335aaf2a4af8","preparationId":"01a09677-6751-763c-ad1e-ebd69605a5a2","modelPreset":"tripo-h-v3.1-standard","providerConfig":{"tripo":{"preset":"tripo-h-v3.1-standard","model":"v3.1-20260211","texture":true,"pbr":true,"textureQuality":"standard","geometryQuality":"standard","faceLimit":100000,"quad":false,"generateParts":false},"manualAi":{"model":"gpt-5-mini","promptVersion":"manual_extract_v1"}},"pageCount":3,"pageRange":{"from":1,"to":3},"maxOutputTokens":4096,"priceVersion":"2026-09-11","priceSnapshotDate":"2026-09-11","amounts":{"tripo":{"currency":"creditMinor","estimatedMinor":3000,"upperBoundMinor":3000,"estimatedDisplay":"30.00 credits","upperBoundDisplay":"30.00 credits","upperBoundLines":[{"code":"multiviewGeneration","description":"多视图生成（标准质量，含纹理与 PBR）","quantity":1,"unit":"generation","unitPriceDecimal":"30","unitPricePerUnits":1,"amountMinor":3000,"amountDisplay":"30.00 credits"}]},"manualAi":{"currency":"usdMicros","estimatedMinor":4885,"upperBoundMinor":8522,"estimatedDisplay":"0.004885 USD","upperBoundDisplay":"0.008522 USD","upperBoundLines":[{"code":"inputTokens","description":"输入 token（页文字/页图 + 提示词开销的保守上界）","quantity":1317,"unit":"tokens","unitPriceDecimal":"0.25","unitPricePerUnits":1000000,"amountMinor":330,"amountDisplay":"0.00033 USD"},{"code":"outputTokens","description":"输出 token（批次数 × 每批上限）","quantity":4096,"unit":"tokens","unitPriceDecimal":"2.00","unitPricePerUnits":1000000,"amountMinor":8192,"amountDisplay":"0.008192 USD"},{"code":"pageImages","description":"页图（扫描页/无文字层页发送视觉输入）","quantity":0,"unit":"images","unitPriceDecimal":"0.01","unitPricePerUnits":1,"amountMinor":0,"amountDisplay":"0.00 USD"}]}},"sendScope":{"tripo":{"views":[{"view":"front","photoId":"01a09677-67c7-76ed-9727-e33171282f19","sha256":"122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586"},{"view":"left","photoId":"01a09677-67da-7295-8842-86d4e36ef862","sha256":"0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890"}],"model":"v3.1-20260211","preset":"tripo-h-v3.1-standard","parameters":{"preset":"tripo-h-v3.1-standard","model":"v3.1-20260211","texture":true,"pbr":true,"textureQuality":"standard","geometryQuality":"standard","faceLimit":100000,"quad":false,"generateParts":false}},"manualAi":{"itemName":"备份测试物品","itemModel":"X100V","model":"gpt-5-mini","promptVersion":"manual_extract_v1","pageFrom":1,"pageTo":3,"pageCount":3,"textPages":[1,2,3],"imagePages":[],"maxOutputTokens":4096},"priceVersion":"2026-09-11","priceSnapshotDate":"2026-09-11","plannedUpperBound":{"tripo":{"currency":"creditMinor","estimatedMinor":3000,"upperBoundMinor":3000,"estimatedDisplay":"30.00 credits","upperBoundDisplay":"30.00 credits","upperBoundLines":[{"code":"multiviewGeneration","description":"多视图生成（标准质量，含纹理与 PBR）","quantity":1,"unit":"generation","unitPriceDecimal":"30","unitPricePerUnits":1,"amountMinor":3000,"amountDisplay":"30.00 credits"}]},"manualAi":{"currency":"usdMicros","estimatedMinor":4885,"upperBoundMinor":8522,"estimatedDisplay":"0.004885 USD","upperBoundDisplay":"0.008522 USD","upperBoundLines":[{"code":"inputTokens","description":"输入 token（页文字/页图 + 提示词开销的保守上界）","quantity":1317,"unit":"tokens","unitPriceDecimal":"0.25","unitPricePerUnits":1000000,"amountMinor":330,"amountDisplay":"0.00033 USD"},{"code":"outputTokens","description":"输出 token（批次数 × 每批上限）","quantity":4096,"unit":"tokens","unitPriceDecimal":"2.00","unitPricePerUnits":1000000,"amountMinor":8192,"amountDisplay":"0.008192 USD"},{"code":"pageImages","description":"页图（扫描页/无文字层页发送视觉输入）","quantity":0,"unit":"images","unitPriceDecimal":"0.01","unitPricePerUnits":1,"amountMinor":0,"amountDisplay":"0.00 USD"}]}},"budgetNotice":"预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准"},"expiresAt":"2026-09-12T16:43:22.908Z","confirmedAt":null,"consumedAt":null,"consumedJobId":null,"createdAt":"2026-09-12T16:33:22.908Z","budgetNotice":"预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准"}',1789231402908,1789230802912,'{"quoteId":"01a09677-67dd-7146-91b1-da403091a711","confirmedAt":"2026-09-12T16:33:22.912Z","sendScope":{"tripo":{"views":[{"view":"front","photoId":"01a09677-67c7-76ed-9727-e33171282f19","sha256":"122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586"},{"view":"left","photoId":"01a09677-67da-7295-8842-86d4e36ef862","sha256":"0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890"}],"model":"v3.1-20260211","preset":"tripo-h-v3.1-standard","parameters":{"preset":"tripo-h-v3.1-standard","model":"v3.1-20260211","texture":true,"pbr":true,"textureQuality":"standard","geometryQuality":"standard","faceLimit":100000,"quad":false,"generateParts":false}},"manualAi":{"itemName":"备份测试物品","itemModel":"X100V","model":"gpt-5-mini","promptVersion":"manual_extract_v1","pageFrom":1,"pageTo":3,"pageCount":3,"textPages":[1,2,3],"imagePages":[],"maxOutputTokens":4096},"priceVersion":"2026-09-11","priceSnapshotDate":"2026-09-11","plannedUpperBound":{"tripo":{"currency":"creditMinor","estimatedMinor":3000,"upperBoundMinor":3000,"estimatedDisplay":"30.00 credits","upperBoundDisplay":"30.00 credits","upperBoundLines":[{"code":"multiviewGeneration","description":"多视图生成（标准质量，含纹理与 PBR）","quantity":1,"unit":"generation","unitPriceDecimal":"30","unitPricePerUnits":1,"amountMinor":3000,"amountDisplay":"30.00 credits"}]},"manualAi":{"currency":"usdMicros","estimatedMinor":4885,"upperBoundMinor":8522,"estimatedDisplay":"0.004885 USD","upperBoundDisplay":"0.008522 USD","upperBoundLines":[{"code":"inputTokens","description":"输入 token（页文字/页图 + 提示词开销的保守上界）","quantity":1317,"unit":"tokens","unitPriceDecimal":"0.25","unitPricePerUnits":1000000,"amountMinor":330,"amountDisplay":"0.00033 USD"},{"code":"outputTokens","description":"输出 token（批次数 × 每批上限）","quantity":4096,"unit":"tokens","unitPriceDecimal":"2.00","unitPricePerUnits":1000000,"amountMinor":8192,"amountDisplay":"0.008192 USD"},{"code":"pageImages","description":"页图（扫描页/无文字层页发送视觉输入）","quantity":0,"unit":"images","unitPriceDecimal":"0.01","unitPricePerUnits":1,"amountMinor":0,"amountDisplay":"0.00 USD"}]}},"budgetNotice":"预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准"},"summary":"已确认发送范围（2026-09-12T16:33:22.912Z）"}',1789230802914,'01a09677-67e3-741e-826a-3d6af9f52523',1789230802908);
CREATE TABLE IF NOT EXISTS "assets" (
    id            TEXT PRIMARY KEY,
    blob_id       TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    item_id       TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    purpose       TEXT NOT NULL
        CHECK (purpose IN ('document', 'photo', 'page_image', 'page_text', 'model', 'release_manifest')),
    original_name TEXT,
    created_at    INTEGER NOT NULL
);
INSERT INTO assets VALUES('01a09677-674f-70f2-bb1c-9538edcf75b5','e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda','01a09677-6737-7665-a84b-335aaf2a4af8','document','manual.pdf',1789230802767);
INSERT INTO assets VALUES('01a09677-675f-7308-aa4f-9565dd03bf41','122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586','01a09677-6737-7665-a84b-335aaf2a4af8','page_image','page.jpg',1789230802783);
INSERT INTO assets VALUES('01a09677-676e-734a-a55a-9f1d68a58820','972f6c241a5c0bfcec4af02a660218f9cd0689d77840fe5cf49a8ffe97edd010','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','page.txt',1789230802798);
INSERT INTO assets VALUES('01a09677-6781-760a-8978-ab3bcec49af9','122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586','01a09677-6737-7665-a84b-335aaf2a4af8','page_image','page.jpg',1789230802817);
INSERT INTO assets VALUES('01a09677-6790-7502-9a37-55a2ef513671','6040550076320303894e351b83bb7844d84ea5431ae7a209947026accd79c79b','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','page.txt',1789230802832);
INSERT INTO assets VALUES('01a09677-67a2-7792-bd74-de0ffa60f833','122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586','01a09677-6737-7665-a84b-335aaf2a4af8','page_image','page.jpg',1789230802850);
INSERT INTO assets VALUES('01a09677-67b2-7077-8464-524fe3411e82','daa48f0d8a240072c9dea69d7fea52229e2c7ab37b5fea1126d02b594d329ccf','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','page.txt',1789230802866);
INSERT INTO assets VALUES('01a09677-67c6-7372-8101-7691cb5524b1','122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586','01a09677-6737-7665-a84b-335aaf2a4af8','photo','sample-photo-front.jpg',1789230802886);
INSERT INTO assets VALUES('01a09677-67d9-7383-b48c-7d3d654643c8','0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890','01a09677-6737-7665-a84b-335aaf2a4af8','photo','sample-photo-left.png',1789230802905);
INSERT INTO assets VALUES('01a09677-67fd-7006-9641-fda220d28222','74b3edaa5bcc408629228edf8365bd691844025c8135c50cc4892bc1acdbb20b','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','manual_extract_batch_0_response.json',1789230802941);
INSERT INTO assets VALUES('01a09677-680a-72da-938d-347ac82fe652','e694fab5444078cb515dd1ac67844a7a6599422116fde2f1e31e984bfc33ea69','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','manual_extract_batch_0.json',1789230802954);
INSERT INTO assets VALUES('01a09677-681b-715b-8eb8-f2108ecb3009','c017464ac0b990e826e3d2c527aeac4b351a169a09b10a4f84fb07959f19e4f1','01a09677-6737-7665-a84b-335aaf2a4af8','page_text','manual_merged_knowledge.json',1789230802971);
INSERT INTO assets VALUES('01a09677-6839-73f4-af93-ec3acdea7db5','a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb','01a09677-6737-7665-a84b-335aaf2a4af8','model','model.glb',1789230803001);
INSERT INTO assets VALUES('01a09677-685a-7542-8e2f-0b852e27cfc5','c2ec7def5b84a2bdad698be1bab9c14f0332255779d6a03c00a9c266b3d12465','01a09677-6737-7665-a84b-335aaf2a4af8','release_manifest',NULL,1789230803034);
CREATE TRIGGER generation_snapshots_immutable
BEFORE UPDATE ON generation_snapshots
BEGIN
    SELECT RAISE(ABORT, 'generation_snapshots 是冻结输入，不允许修改');
END;
CREATE TRIGGER manual_releases_immutable
BEFORE UPDATE ON manual_releases
BEGIN
    SELECT RAISE(ABORT, 'manual_releases 是已发布快照，不允许修改');
END;
CREATE TRIGGER manual_releases_no_delete
BEFORE DELETE ON manual_releases
BEGIN
    SELECT RAISE(ABORT, 'manual_releases 是已发布快照，不允许删除（归档用 draft/物品字段）');
END;
CREATE TRIGGER provider_attempts_remote_task_id_monotonic
BEFORE UPDATE OF remote_task_id ON provider_attempts
WHEN OLD.remote_task_id IS NOT NULL
     AND (NEW.remote_task_id IS NULL OR NEW.remote_task_id <> OLD.remote_task_id)
BEGIN
    SELECT RAISE(ABORT, 'provider_attempts.remote_task_id 已存在：只允许 null→值或同值，不允许覆盖或清空');
END;
CREATE TRIGGER quotes_snapshot_immutable
BEFORE UPDATE OF item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset,
                 provider_config, price_version, price_snapshot_date, page_count,
                 max_output_tokens, quote_json, expires_at, created_at ON quotes
BEGIN
    SELECT RAISE(ABORT, 'quotes 是报价快照，不允许修改');
END;
CREATE TRIGGER quotes_confirmation_frozen
BEFORE UPDATE OF confirmed_at ON quotes
WHEN OLD.confirmed_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'quotes 的确认已记录：不允许再次修改或清除');
END;
CREATE TRIGGER quotes_consumption_frozen
BEFORE UPDATE OF consumed_at ON quotes
WHEN OLD.consumed_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'quotes 已被任务消费：不允许改写');
END;
CREATE INDEX sessions_admin ON sessions (admin_id);
CREATE INDEX sessions_expires ON sessions (expires_at);
CREATE INDEX items_created ON items (created_at DESC);
CREATE INDEX documents_item ON documents (item_id);
CREATE INDEX preparations_document ON preparations (document_id, state);
CREATE INDEX photos_item ON photos (item_id, view);
CREATE INDEX generation_snapshots_item ON generation_snapshots (item_id, created_at DESC);
CREATE INDEX jobs_item ON jobs (item_id, created_at DESC);
CREATE INDEX jobs_status ON jobs (status, updated_at);
CREATE INDEX job_stages_job ON job_stages (job_id);
CREATE INDEX job_stages_due ON job_stages (status, next_run_at);
CREATE INDEX provider_attempts_job ON provider_attempts (job_id);
CREATE INDEX provider_attempts_stage ON provider_attempts (stage_id);
CREATE INDEX idempotency_records_resource ON idempotency_records (resource_id);
CREATE INDEX cost_ledger_snapshot ON cost_ledger (snapshot_id, provider, state);
CREATE INDEX cost_ledger_attempt ON cost_ledger (attempt_id);
CREATE INDEX model_revisions_item ON model_revisions (item_id, created_at DESC);
CREATE INDEX manual_drafts_item ON manual_drafts (item_id, created_at DESC);
CREATE INDEX manual_releases_item ON manual_releases (item_id, created_at DESC);
CREATE INDEX manual_releases_draft ON manual_releases (draft_id);
CREATE INDEX audit_events_entity ON audit_events (entity_type, entity_id, created_at);
CREATE INDEX audit_events_created ON audit_events (created_at);
CREATE UNIQUE INDEX provider_attempts_unresolved_stage
    ON provider_attempts (stage_id)
    WHERE submit_state IN ('intent', 'submitting', 'unknown');
CREATE UNIQUE INDEX photos_item_view_unique ON photos (item_id, view);
CREATE INDEX job_stage_deps_depends_on ON job_stage_deps (depends_on_stage_id);
CREATE INDEX quotes_item ON quotes (item_id, created_at DESC);
CREATE INDEX quotes_expires ON quotes (expires_at);
CREATE UNIQUE INDEX cost_ledger_active_reservation
    ON cost_ledger (snapshot_id, provider) WHERE state = 'reserved';
CREATE INDEX assets_item ON assets (item_id, purpose);
CREATE INDEX assets_blob ON assets (blob_id);
COMMIT;
