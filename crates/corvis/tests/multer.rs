//! File uploads: parity with the Express platform + multer 2.2 + busboy 1.6.
//! Expected bodies and statuses were captured from a real Express app with the same routes
//! (except the magic-number case of `FileTypeValidator`, which follows the validator source because
//! it needs the optional `file-type` package).

use std::collections::HashMap;

use corvis::multer::{DiskStorageOptions, Limits, MulterOptions, disk_storage, memory_storage};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{
    AnyFilesInterceptor, FileFieldsInterceptor, FileInterceptor, FileTypeValidator, FilesInterceptor, MaxFileSizeValidator, MulterField,
    MulterModule, NoFilesInterceptor, ParseFilePipe, ParseFilePipeBuilder, UploadedFile,
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

fn upload_dir() -> String {
    let d = std::env::temp_dir().join(format!("corvis-multer-test-{}", std::process::id()));
    d.to_string_lossy().into_owned()
}

#[controller("files")]
pub struct FilesController;

#[routes]
impl FilesController {
    #[post("single")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn single(&self, #[uploaded_file] file: Option<UploadedFile>, #[body] body: Value) -> Value {
        json!({
            "body": body,
            "file": file.map(|f| json!({
                "fieldname": f.fieldname, "originalname": f.originalname, "encoding": f.encoding,
                "mimetype": f.mimetype, "size": f.size,
                "content": String::from_utf8_lossy(f.buffer.as_deref().unwrap_or_default()),
            })),
        })
    }

    /// Returning the file as is (`buffer` becomes `{ type: 'Buffer', data: [...] }`).
    #[post("raw")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn raw(&self, #[uploaded_file] file: UploadedFile) -> UploadedFile {
        file
    }

    #[post("limited")]
    #[use_interceptors(FileInterceptor::new("file").limits(Limits::new().file_size(5).field_size(4).fields(2)))]
    fn limited(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value {
        json!({ "size": file.map(|f| f.size) })
    }

    #[post("array")]
    #[use_interceptors(FilesInterceptor::new("photos").max_count(2))]
    fn array(&self, #[uploaded_files] files: Vec<UploadedFile>) -> Value {
        json!(files.iter().map(|f| f.originalname.clone()).collect::<Vec<_>>())
    }

    #[post("fields")]
    #[use_interceptors(FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")]))]
    fn fields(&self, #[uploaded_files] files: HashMap<String, Vec<UploadedFile>>) -> Value {
        let mut keys: Vec<_> = files.iter().map(|(k, v)| format!("{k}:{}", v.len())).collect();
        keys.sort();
        json!(keys)
    }

    #[post("any")]
    #[use_interceptors(AnyFilesInterceptor::new())]
    fn any(&self, #[uploaded_files] files: Vec<UploadedFile>, #[body] body: Value) -> Value {
        json!({ "files": files.iter().map(|f| f.fieldname.clone()).collect::<Vec<_>>(), "body": body })
    }

    #[post("none")]
    #[use_interceptors(NoFilesInterceptor::new())]
    fn none(&self, #[body] body: Value) -> Value {
        body
    }

    #[post("disk")]
    #[use_interceptors(FileInterceptor::new("file").storage(disk_storage(
        DiskStorageOptions::new()
            .destination(upload_dir())
            .filename(|_req, file| Ok(format!("fixed-{}", file.originalname))),
    )))]
    fn disk(&self, #[uploaded_file] file: UploadedFile) -> Value {
        let content = std::fs::read(file.path.as_deref().unwrap()).unwrap();
        json!({ "file": file, "content": String::from_utf8_lossy(&content) })
    }

    #[post("filtered")]
    #[use_interceptors(FileInterceptor::new("file").options(MulterOptions::new().file_filter(|_req, file| {
        if file.mimetype == "application/x-evil" {
            return Err(BadRequestException::new("Only images are allowed"));
        }
        Ok(file.mimetype.starts_with("image/"))
    })))]
    fn filtered(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value {
        json!({ "accepted": file.is_some() })
    }

    #[post("validated")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn validated(
        &self,
        #[uploaded_file(ParseFilePipe::new()
            .validator(MaxFileSizeValidator::new(1000))
            .validator(FileTypeValidator::new("image/png")))]
        file: UploadedFile,
    ) -> Value {
        json!({ "ok": file.originalname })
    }

    #[post("optional")]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn optional(
        &self,
        #[uploaded_file(ParseFilePipeBuilder::new()
            .add_max_size_validator(MaxFileSizeValidator::new(3).message("too big"))
            .build(ParseFilePipe::new().file_is_required(false).error_http_status_code(422)))]
        file: Option<UploadedFile>,
    ) -> Value {
        json!({ "has": file.is_some() })
    }
}

#[module(controllers = [FilesController])]
pub struct FilesModule;

#[controller("defaults")]
pub struct DefaultsController;

#[routes]
impl DefaultsController {
    #[post]
    #[use_interceptors(FileInterceptor::new("file"))]
    fn upload(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value {
        json!({ "size": file.map(|f| f.size) })
    }
}

#[module(imports = [MulterModule::register(MulterOptions::new().limits(Limits::new().file_size(3)).storage(memory_storage()))], controllers = [DefaultsController])]
pub struct DefaultsModule;

#[module(imports = [FilesModule, DefaultsModule])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

fn raw_multipart(app: &CorvisApplication, path: &str, content_type: &str, body: &str) -> corvis::testing::TestRequest {
    request(app).post(path).header("content-type", content_type).body(body.to_string())
}

#[corvis::test]
async fn single_file_and_fields() {
    let app = app().await;
    let r = request(&app)
        .post("/files/single")
        .field("title", "Hola")
        .field("tags[]", "a")
        .field("tags[]", "b")
        .field("user[name]", "Ana")
        .attach("file", "contenido", "notas.txt")
        .send()
        .await;
    r.expect_status(201).expect_json(json!({
        "body": { "title": "Hola", "tags": ["a", "b"], "user": { "name": "Ana" } },
        "file": { "fieldname": "file", "originalname": "notas.txt", "encoding": "7bit", "mimetype": "text/plain", "size": 9, "content": "contenido" },
    }));

    // No file: `@UploadedFile()` is undefined and the body is `{}`... (an empty multipart form)
    let r = request(&app).post("/files/single").field("a", "1").send().await;
    r.expect_status(201).expect_json(json!({ "body": { "a": "1" }, "file": null }));

    // Not multipart: multer does nothing.
    let r = request(&app).post("/files/single").json(&json!({ "x": 1 })).send().await;
    r.expect_status(201).expect_json(json!({ "body": { "x": 1 }, "file": null }));
}

#[corvis::test]
async fn returned_file_serializes_like_express() {
    let app = app().await;
    let r = request(&app).post("/files/raw").attach("file", "hi", "a.txt").send().await;
    r.expect_status(201);
    assert_eq!(
        r.text(),
        r#"{"fieldname":"file","originalname":"a.txt","encoding":"7bit","mimetype":"text/plain","buffer":{"type":"Buffer","data":[104,105]},"size":2}"#
    );
}

#[corvis::test]
async fn multer_errors_map_to_http_exceptions() {
    let app = app().await;
    // Unexpected field.
    let r = request(&app).post("/files/single").attach("other", "x", "a.txt").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Unexpected field - other", "error": "Bad Request", "statusCode": 400 }));
    // Two files for a single-file field.
    let r = request(&app).post("/files/single").attach("file", "x", "a.txt").attach("file", "y", "b.txt").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Unexpected field - file", "error": "Bad Request", "statusCode": 400 }));
    // File too large (a file that reaches the limit is rejected, like busboy).
    let r = request(&app).post("/files/limited").attach("file", "12345", "a.txt").send().await;
    r.expect_status(413).expect_json(json!({ "message": "File too large", "error": "Payload Too Large", "statusCode": 413 }));
    let r = request(&app).post("/files/limited").attach("file", "1234", "a.txt").send().await;
    r.expect_status(201).expect_json(json!({ "size": 4 }));
    // Field value too long / too many fields.
    let r = request(&app).post("/files/limited").field("note", "abcd").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Field value too long - note", "error": "Bad Request", "statusCode": 400 }));
    let r = request(&app).post("/files/limited").field("a", "1").field("b", "2").field("c", "3").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Too many fields", "error": "Bad Request", "statusCode": 400 }));
    // Too many files for FilesInterceptor(maxCount 2).
    let r = request(&app)
        .post("/files/array")
        .attach("photos", "1", "1.txt")
        .attach("photos", "2", "2.txt")
        .attach("photos", "3", "3.txt")
        .send()
        .await;
    r.expect_status(400).expect_json(json!({ "message": "Unexpected field - photos", "error": "Bad Request", "statusCode": 400 }));
    // NoFilesInterceptor rejects files.
    let r = request(&app).post("/files/none").field("a", "1").attach("doc", "x", "a.txt").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Unexpected field - doc", "error": "Bad Request", "statusCode": 400 }));
}

#[corvis::test]
async fn malformed_bodies() {
    let app = app().await;
    let r = raw_multipart(&app, "/files/single", "multipart/form-data", "x").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Multipart: Boundary not found", "error": "Bad Request", "statusCode": 400 }));
    let r = raw_multipart(
        &app,
        "/files/single",
        "multipart/form-data; boundary=XX",
        "--XX\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nhola",
    )
    .send()
    .await;
    r.expect_status(400).expect_json(json!({ "message": "Multipart: Unexpected end of form", "error": "Bad Request", "statusCode": 400 }));
    let r = raw_multipart(&app, "/files/single", "multipart/form-data; boundary=XX", "--XX\r\nNo Colon\r\n\r\nx\r\n--XX--").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Multipart: Malformed part header", "error": "Bad Request", "statusCode": 400 }));
    let r = raw_multipart(
        &app,
        "/files/single",
        "multipart/form-data; boundary=XX",
        "--XX\r\nContent-Disposition: form-data\r\n\r\nx\r\n--XX--",
    )
    .send()
    .await;
    r.expect_status(400).expect_json(json!({ "message": "Field name missing", "error": "Bad Request", "statusCode": 400 }));
    // Not form-data: busboy throws a plain Error -> 500.
    let r = raw_multipart(&app, "/files/single", "multipart/mixed; boundary=XX", "--XX--").send().await;
    r.expect_status(500).expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));
}

#[corvis::test]
async fn files_array_fields_and_any() {
    let app = app().await;
    let r = request(&app).post("/files/array").attach("photos", "1", "1.png").attach("photos", "2", "2.png").send().await;
    r.expect_status(201).expect_json(json!(["1.png", "2.png"]));
    let r = request(&app).post("/files/array").field("x", "1").send().await;
    r.expect_status(201).expect_json(json!([]));

    let r = request(&app)
        .post("/files/fields")
        .attach("avatar", "a", "a.png")
        .attach("docs", "1", "1.pdf")
        .attach("docs", "2", "2.pdf")
        .send()
        .await;
    r.expect_status(201).expect_json(json!(["avatar:1", "docs:2"]));
    let r = request(&app).post("/files/fields").attach("avatar", "a", "a.png").attach("avatar", "b", "b.png").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Unexpected field - avatar", "error": "Bad Request", "statusCode": 400 }));

    let r = request(&app).post("/files/any").attach("x", "1", "1.txt").field("k", "v").attach("y", "2", "2.txt").send().await;
    r.expect_status(201).expect_json(json!({ "files": ["x", "y"], "body": { "k": "v" } }));

    let r = request(&app).post("/files/none").field("a", "1").field("a", "2").send().await;
    r.expect_status(201).expect_json(json!({ "a": ["1", "2"] }));
}

#[corvis::test]
async fn disk_storage_writes_the_file() {
    let app = app().await;
    let r = request(&app).post("/files/disk").attach("file", "en disco", "doc.txt").send().await;
    r.expect_status(201);
    let v = r.json_value();
    let dir = upload_dir();
    let expected_path = corvis::multer::node_path_join(&dir, "fixed-doc.txt");
    assert_eq!(
        serde_json::to_string(&v["file"]).unwrap(),
        serde_json::to_string(&json!({
            "fieldname": "file", "originalname": "doc.txt", "encoding": "7bit", "mimetype": "text/plain",
            "destination": dir, "filename": "fixed-doc.txt", "path": expected_path, "size": 8,
        }))
        .unwrap()
    );
    assert_eq!(v["content"], "en disco");
    let _ = std::fs::remove_dir_all(upload_dir());
}

#[corvis::test]
async fn file_filter() {
    let app = app().await;
    let r = request(&app).post("/files/filtered").attach("file", PNG.to_vec(), "a.png").send().await;
    r.expect_status(201).expect_json(json!({ "accepted": true }));
    let r = request(&app).post("/files/filtered").attach("file", "x", "a.txt").send().await;
    r.expect_status(201).expect_json(json!({ "accepted": false }));
    let r = request(&app).post("/files/filtered").attach_with_type("file", "x", "a.bin", "application/x-evil").send().await;
    r.expect_status(400).expect_json(json!({ "message": "Only images are allowed", "error": "Bad Request", "statusCode": 400 }));
}

#[corvis::test]
async fn parse_file_pipe() {
    let app = app().await;
    let r = request(&app).post("/files/validated").field("a", "1").send().await;
    r.expect_status(400).expect_json(json!({ "message": "File is required", "error": "Bad Request", "statusCode": 400 }));
    let r = request(&app).post("/files/validated").attach("file", PNG.to_vec(), "a.png").send().await;
    r.expect_status(201).expect_json(json!({ "ok": "a.png" }));
    // The declared mimetype says png, the content does not (magic numbers).
    let r = request(&app).post("/files/validated").attach_with_type("file", "not a png", "a.png", "image/png").send().await;
    r.expect_status(400).expect_json(json!({
        "message": "Validation failed (current file type is image/png, expected type is image/png)",
        "error": "Bad Request", "statusCode": 400,
    }));
    let big = vec![b'a'; 1500];
    let r = request(&app).post("/files/validated").attach("file", big, "a.png").send().await;
    r.expect_status(400).expect_json(json!({
        "message": "Validation failed (current file size is 1500, expected size is less than 1000)",
        "error": "Bad Request", "statusCode": 400,
    }));

    let r = request(&app).post("/files/optional").field("a", "1").send().await;
    r.expect_status(201).expect_json(json!({ "has": false }));
    let r = request(&app).post("/files/optional").attach("file", "1234", "a.txt").send().await;
    r.expect_status(422).expect_json(json!({ "message": "too big", "error": "Unprocessable Entity", "statusCode": 422 }));
}

#[corvis::test]
async fn module_options_are_defaults() {
    let app = app().await;
    let r = request(&app).post("/defaults").attach("file", "12", "a.txt").send().await;
    r.expect_status(201).expect_json(json!({ "size": 2 }));
    let r = request(&app).post("/defaults").attach("file", "123", "a.txt").send().await;
    r.expect_status(413).expect_json(json!({ "message": "File too large", "error": "Payload Too Large", "statusCode": 413 }));
}
