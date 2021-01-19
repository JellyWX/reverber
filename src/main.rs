use actix::{Actor, StreamHandler};
use actix_files::Files;
use actix_web::{
    error, get, middleware::Logger, post, web, App, Error, HttpRequest, HttpResponse, HttpServer,
    Result,
};
use actix_web_actors::ws;

use dotenv::dotenv;

use tera::Tera;

use tokio::process::Command;

use env_logger::Env;

use actix_web::http::header;
use rand::prelude::IteratorRandom;
use rand::rngs::OsRng;
use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use std::process::Stdio;

#[derive(Deserialize)]
struct ReverbRequest {
    url: String,
    delay: u8,
    decay: f64,
    out_gain: f64,
    tempo: f64,
}

struct ReadyWs;

impl Actor for ReadyWs {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ReadyWs {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            _ => (),
        }
    }
}

#[get("/")]
async fn index(tmpl: web::Data<Tera>) -> Result<HttpResponse, Error> {
    let s: String = tmpl
        .render("index.html", &tera::Context::new())
        .map_err(|_| error::ErrorInternalServerError("Template error"))?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

#[get("/ws")]
async fn waiting_ws(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    let resp = ws::start_with_addr(ReadyWs {}, &req, stream)?;

    //tokio::spawn(async move {});

    Ok(resp.1)
}

async fn waiting(
    req: HttpRequest,
    tmpl: web::Data<Tera>,
    pool: web::Data<SqlitePool>,
) -> Result<HttpResponse, Error> {
    let route = req.match_info().get("route").unwrap();

    let d = sqlx::query(
        "
SELECT data FROM routes WHERE route = ?
        ",
    )
    .bind(&route)
    .fetch_one(&**pool)
    .await
    .map_err(|_| HttpResponse::InternalServerError().body("Database error"))?;

    if let Some(data) = d.get::<Option<Vec<u8>>, &str>("data") {
        Ok(HttpResponse::Ok().content_type("audio/mpeg").body(data))
    } else {
        let mut ctx = tera::Context::new();
        ctx.insert("route", route);

        let s: String = tmpl
            .render("wait.html", &ctx)
            .map_err(|_| error::ErrorInternalServerError("Template error"))?;

        Ok(HttpResponse::Ok().content_type("text/html").body(s))
    }
}

#[post("/reverb")]
async fn reverb_route(
    req: HttpRequest,
    form: web::Form<ReverbRequest>,
    pool: web::Data<SqlitePool>,
) -> Result<HttpResponse, Error> {
    let route = random_route(8);
    let route_clone = route.clone();

    tokio::spawn(async move {
        reverb(form, route_clone, pool.clone()).await;
    });

    Ok(HttpResponse::TemporaryRedirect()
        .header(header::LOCATION, req.url_for("queue", &[&route])?.as_str())
        .content_type("text/html")
        .body("Redirected"))
}

async fn reverb(form: web::Form<ReverbRequest>, route: String, pool: web::Data<SqlitePool>) {
    sqlx::query(
        "
INSERT INTO routes (route) VALUES (?)
        ",
    )
    .bind(&route)
    .execute(&**pool)
    .await
    .unwrap();

    let ytdl = std::process::Command::new("youtube-dl")
        .arg(&form.url)
        .arg("-o")
        .arg("-")
        .arg("-q")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let reverbed = Command::new("ffmpeg")
        .arg("-i")
        .arg("pipe:")
        .arg("-loglevel")
        .arg("error")
        .arg("-vn")
        .arg("-filter:a")
        .arg(format!(
            "aecho=1.0:{out_gain}:{delay}:{decay},asetrate=48000*{tempo},aresample=48000*{tempo},atempo={tempo}",
            out_gain = form.out_gain,
            delay = form.delay,
            decay = form.decay,
            tempo = form.tempo,
        ))
        .arg("-f")
        .arg("mp3")
        .arg("pipe:")
        .stdin(ytdl.stdout.unwrap())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .await
        .unwrap();

    sqlx::query(
        "
UPDATE routes SET data = ? WHERE route = ?
        ",
    )
    .bind(reverbed.stdout)
    .bind(&route)
    .execute(&**pool)
    .await
    .unwrap();
}

fn random_route(len: usize) -> String {
    let mut rng: OsRng = Default::default();

    (0..len)
        .map(|_| {
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
                .chars()
                .choose(&mut rng)
                .unwrap()
                .to_owned()
                .to_string()
        })
        .collect::<Vec<String>>()
        .join("")
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().unwrap();

    env_logger::from_env(Env::default().default_filter_or("info")).init();
    let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();

    sqlx::query(
        "
CREATE TABLE routes(
    route TEXT,
    data BLOB
)
        ",
    )
    .execute(&pool)
    .await
    .expect("Could not set up SQLite");

    HttpServer::new(move || {
        let tera = Tera::new(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*")).unwrap();

        App::new()
            .data(tera)
            .data(pool.clone())
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .service(index)
            .service(reverb_route)
            .service(waiting_ws)
            .service(web::resource("/queue/{route}").name("queue").to(waiting))
            .service(Files::new("/static", "./static/").show_files_listing())
    })
    .bind("127.0.0.1:5000")?
    .run()
    .await
}
