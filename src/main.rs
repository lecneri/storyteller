mod auth;
mod chaves;
mod cli_login;
mod config;
mod conta;
mod creditos;
mod crianca;
mod elevenlabs;
mod email;
mod estantes;
mod exportar;
mod familia;
mod historias;
mod imprimir;
mod fila_imagens;
mod imagem;
mod ia;
mod jobs;
mod limpeza;
mod midia;
mod musica;
mod narracao;
mod recorte;
mod partilha;
mod personagens;
mod player;
mod segredos;
mod sistema;
mod tts;
mod util;
mod video;
mod web;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use axum::{extract::State, http::StatusCode, middleware, routing::{get, post}, Json, Router};
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    db: PgPool,
    /// Cookie só via HTTPS (ligar em produção).
    cookie_seguro: bool,
    data_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "storyteller=info".into()))
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL não definida")?;
    let bind: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("BIND_ADDR inválido")?;
    let data_dir = PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".into()));
    tokio::fs::create_dir_all(&data_dir)
        .await
        .with_context(|| format!("não consegui criar {}", data_dir.display()))?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("falha ao conectar no Postgres")?;
    sqlx::migrate!("./migrations").run(&db).await.context("falha nas migrações")?;
    tracing::info!("migrações aplicadas");
    auth::bootstrap_admin(&db).await.context("falha ao criar o admin inicial")?;

    let cookie_seguro = std::env::var("COOKIE_SEGURO").map(|v| v == "true").unwrap_or(false);
    let state = AppState { db, cookie_seguro, data_dir };
    if let Err(e) = config::carregar(&state).await {
        tracing::warn!("não carreguei as configurações do sistema: {e:#}");
    }
    jobs::recuperar(&state).await;
    tokio::spawn(jobs::worker(state.clone()));
    fila_imagens::recuperar(&state).await;
    for fila in imagem::todas_filas() {
        tokio::spawn(fila_imagens::trabalhador(state.clone(), fila));
    }
    tokio::spawn(limpeza::agendar(state.clone()));
    let app = Router::new()
        .route("/saude", get(saude))
        .route("/creditos", get(creditos::ver))
        .route("/", get(estantes::inicio))
        .route("/leitores", get(web::painel))
        .route("/estantes", get(|| async { axum::response::Redirect::to("/") }).post(estantes::criar))
        .route("/estantes/{slug}", get(estantes::abrir).post(estantes::salvar))
        .route("/estantes/{slug}/historias", post(estantes::incluir))
        .route("/estantes/{slug}/historias/{historia}/retirar", post(estantes::retirar))
        .route("/entrar", get(web::pagina_entrar).post(web::entrar))
        .route("/sair", post(web::sair))
        .route("/conta", get(conta::ver).post(conta::salvar))
        .route("/conta/dados", get(conta::meus_dados))
        .route("/conta/apagar", post(conta::apagar_conta))
        .route("/instalar", get(conta::pagina_instalar).post(conta::instalar))
        .route("/esqueci", get(conta::pagina_esqueci).post(conta::esqueci))
        .route("/redefinir/{token}", get(conta::pagina_redefinir).post(conta::redefinir))
        .route("/exportar", get(exportar::exportar))
        .route("/leitores/novo", post(web::criar_leitor))
        .route("/personagens", get(personagens::lista).post(personagens::criar))
        .route("/personagens/ia", post(personagens::criar_com_ia))
        .route("/personagens/descrever-foto", post(personagens::descrever_foto))
        .route("/personagens/de-secundario", post(historias::salvar_secundario))
        .route("/personagens/{slug}", get(personagens::detalhe).post(personagens::salvar_ficha))
        .route("/personagens/{slug}/variantes", post(personagens::criar_variante))
        .route("/personagens/{slug}/variantes/{vslug}/atual", post(personagens::marcar_atual))
        .route("/personagens/{slug}/variantes/{vslug}/refs", post(personagens::adicionar_refs))
        .route("/personagens/{slug}/variantes/{vslug}/tirar-ref/{ref_id}", post(personagens::tirar_ref))
        .route("/personagens/{slug}/variantes/{vslug}/recortar/{ref_id}", post(personagens::recortar_ref))
        .route("/personagens/{slug}/fotos", post(personagens::fotos::enviar_fotos))
        .route("/personagens/{slug}/fotos/{foto}/tirar", post(personagens::fotos::tirar_foto))
        .route("/personagens/{slug}/variantes/{vslug}/arquivar", post(personagens::alternar_arquivo))
        .route("/personagens/{slug}/variantes/{vslug}/editar", post(personagens::editar_variante))
        .route("/personagens/{slug}/variantes/{vslug}/gerar-ref", post(personagens::gerar_ref))
        .route("/midia/{id}", get(midia::servir))
        .route("/midia/{id}/quadro", get(imprimir::quadro))
        .route("/imprimir/{id}", get(imprimir::pagina))
        .route("/historias/{id}/exportar", get(video::pagina))
        .route("/historias/{id}/video", post(video::fazer))
        .route("/historias/{id}/video/estado", get(video::estado))
        .route("/historias/{id}/video/baixar", get(video::baixar))
        .route("/historias/{id}/video/ver", get(video::ver))
        .route("/historias", get(historias::lista).post(historias::criar))
        .route("/historias/{id}", get(historias::abrir))
        .route("/historias/{id}/regerar", post(historias::regerar))
        .route("/historias/{id}/importar", post(historias::importar))
        .route("/historias/{id}/ajuda-ia", post(historias::pedir_ajuda))
        .route("/historias/{id}/ajuda-ia/importar", post(historias::importar_ajuda))
        .route("/historias/{id}/ajuda-ia/{job}", get(historias::ver_ajuda))
        .route("/historias/{id}/apagar", post(historias::apagar))
        .route("/mencoes", get(historias::mencoes))
        .route("/historias/{id}/memoria", post(historias::gerar_memoria_rota))
        .route("/historias/{id}/memoria/salvar", post(historias::salvar_memoria))
        .route("/historias/{id}/conteudo", post(historias::salvar))
        .route("/historias/{id}/imagens", get(historias::imagens::estado).post(historias::imagens::pedir))
        .route("/historias/{id}/imagens/parar", post(historias::imagens::parar))
        .route("/historias/{id}/midia", post(historias::enviar_midia))
        .route("/historias/{id}/capa", post(historias::definir_capa))
        .route("/historias/{id}/imagem-ia", post(historias::imagens::pedir_uma))
        .route("/historias/{id}/fonte-imagem", post(historias::imagens::definir_fonte))
        .route("/imagens/fila", get(fila_imagens::estado))
        .route("/historias/{id}/bloco-audio", post(narracao::audio_bloco))
        .route("/historias/{id}/narrar", post(narracao::narrar))
        .route("/historias/{id}/gravacao", post(narracao::gravar))
        .route("/historias/{id}/musica", post(musica::enviar))
        .route("/historias/{id}/musica/ajustar", post(musica::ajustar))
        .route("/historias/{id}/musica/modelo", post(musica::usar_modelo))
        .route("/musicas-modelo/{id}", get(musica::ouvir_modelo))
        .route("/sistema/musicas", get(musica::listar_modelos).post(musica::criar_modelo))
        .route("/sistema/musicas/{id}/apagar", post(musica::apagar_modelo))
        .route("/historias/{id}/narracao", get(narracao::estado))
        .route("/ler/{id}", get(player::ler))
        .route("/crianca", get(crianca::inicio))
        .route("/sair-leitura", get(crianca::pagina_sair).post(crianca::sair))
        .route("/leitores/{id}/modo-leitura", post(crianca::entrar))
        .route("/leitores/{id}/estantes", post(crianca::salvar_estantes))
        .route("/familia/pin", post(crianca::definir_pin))
        .route("/familia", get(familia::ver))
        .route("/familia/config", post(familia::salvar_config))
        .route("/familia/convites", post(familia::criar_convite))
        .route("/familia/convites/{hash}/revogar", post(familia::revogar_convite))
        .route("/familia/membros/{usuario}/papel", post(familia::mudar_papel))
        .route("/familia/membros/{usuario}/remover", post(familia::remover_membro))
        .route("/familia/chaves", post(familia::salvar_chave))
        .route("/familia/chaves/{provedor}/remover", post(familia::remover_chave))
        .route("/convite/{token}", get(familia::abrir_convite).post(familia::aceitar_convite))
        .route("/trocar-familia", post(familia::trocar))
        .route("/familia/conexoes", post(partilha::conectar))
        .route("/familia/conexoes/{familia}/aceitar", post(partilha::aceitar))
        .route("/familia/conexoes/{familia}/remover", post(partilha::desconectar))
        .route("/compartilhar/{tipo}/{id}", get(partilha::ver).post(partilha::salvar))
        .route("/compartilhar/{tipo}/{id}/link", post(partilha::criar_link))
        .route("/compartilhar/{tipo}/{id}/link/{hash}/revogar", post(partilha::revogar_link))
        .route("/compartilhar/{tipo}/{id}/link/{hash}/vozes", post(partilha::alternar_vozes_link))
        .route("/historias/{id}/copiar", post(partilha::copiar_historia))
        .route("/p/{token}", get(partilha::publico))
        .route("/p/{token}/h/{historia}", get(partilha::publico_historia))
        .route("/p/{token}/midia/{id}", get(partilha::publico_midia))
        .route("/p/{token}/bloco-audio", post(partilha::publico_audio))
        .route("/sistema", get(sistema::ver))
        .route("/sistema/familias", post(sistema::criar_familia))
        .route("/sistema/familias/{id}", get(sistema::ver_familia).post(sistema::salvar_familia))
        .route("/sistema/familias/{id}/convite", post(sistema::convite_admin))
        .route("/sistema/chaves", post(sistema::salvar_chave))
        .route("/sistema/chaves/{provedor}/remover", post(sistema::remover_chave))
        .route("/sistema/config", post(sistema::salvar_config))
        .route("/chaves/{escopo}/{provedor}/modelos", post(chaves::modelos))
        .route("/chaves/{escopo}/{provedor}/modelo", post(chaves::salvar_modelo))
        .route("/chaves/{escopo}/{provedor}/testar", post(chaves::testar))
        .route("/sistema/limpar", post(sistema::limpar_agora))
        .route("/sistema/email-teste", post(sistema::email_teste))
        .route("/sistema/cli/{nome}/iniciar", post(sistema::cli_iniciar))
        .route("/sistema/cli/{nome}/estado", get(sistema::cli_estado))
        .route("/sistema/cli/{nome}/codigo", post(sistema::cli_codigo))
        .route("/sistema/cli/{nome}/testar", post(sistema::cli_testar))
        .route("/vozes", get(narracao::vozes))
        .route("/tts/previa", post(narracao::previa))
        .route("/personagens/{slug}/voz", post(narracao::salvar_voz_personagem))
        .route("/personagens/{slug}/virar-variante", post(personagens::virar_variante))
        .route("/personagens/{slug}/vozes-extras", get(narracao::vozes_extras).post(narracao::salvar_vozes_extras))
        .route("/familia/narrador", post(narracao::salvar_narrador))
        .layer(axum::extract::DefaultBodyLimit::max(midia::LIMITE_BYTES * 4))
        .fallback_service(ServeDir::new("static"))
        .layer(middleware::from_fn_with_state(state.clone(), crianca::trava))
        .layer(middleware::from_fn(web::checa_origem))
        .layer(middleware::from_fn(web::paginas_de_erro))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("ouvindo em http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn saude(State(st): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    match sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&st.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true, "banco": "ok" }))),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "ok": false, "banco": e.to_string() })),
        ),
    }
}
