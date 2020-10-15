use actix_files::Files;
use actix_web::{
    error, get, middleware::Logger, post, web, App, Error, HttpResponse, HttpServer, Result,
};

use bytes::Bytes;

use dotenv::dotenv;

use tera::Tera;

use tokio::process::Command;

use env_logger::Env;

use serde::Deserialize;
use std::process::Stdio;

use futures::future::ok;
use futures::stream::once;

#[derive(Deserialize)]
struct JsonRequest {
    url: String,
}

#[get("/")]
async fn index(tmpl: web::Data<Tera>) -> Result<HttpResponse, Error> {
    let s: String = tmpl
        .render("index.html", &tera::Context::new())
        .map_err(|_| error::ErrorInternalServerError("Template error"))?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

#[post("/reverb")]
async fn reverb(form: web::Json<JsonRequest>) -> Result<HttpResponse, Error> {
    let ytdl = std::process::Command::new("youtube-dl")
        .arg(&form.url)
        .arg("-o")
        .arg("-")
        .arg("-q")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| error::ErrorInternalServerError("Could not run youtube-dl"))?;

    let reverbed = Command::new("ffmpeg")
        .arg("-i")
        .arg("pipe:")
        .arg("-loglevel")
        .arg("error")
        .arg("-vn")
        .arg("-filter:a")
        .arg("aecho=1.0:0.7:20:0.5,asetrate=48000*0.85,aresample=48000*0.85,atempo=0.85")
        .arg("-b:a")
        .arg("48000")
        .arg("-f")
        .arg("ogg")
        .arg("pipe:")
        .stdin(ytdl.stdout.ok_or(error::ErrorInternalServerError(
            "YouTube-DL stdout could not be accessed",
        ))?)
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .await
        .map_err(|_| error::ErrorInternalServerError("Could not run ffmpeg"))?;

    let result = base64::encode(reverbed.stdout);

    let body = once(ok::<_, Error>(Bytes::from(result)));

    Ok(HttpResponse::Ok()
        .content_type("application/base64")
        .streaming(body))
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().unwrap();

    env_logger::from_env(Env::default().default_filter_or("info")).init();

    HttpServer::new(move || {
        let tera = Tera::new(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*")).unwrap();

        App::new()
            .data(tera)
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .service(index)
            .service(reverb)
            .service(Files::new("/static", "./static/").show_files_listing())
    })
    .bind("127.0.0.1:5000")?
    .run()
    .await
}
