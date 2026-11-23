//! Importações com erro: a resposta de outra IA colada (história nova, ajuda do editor, memória) que não deu para usar
//! fica guardada com o erro, para investigar em Sistema › Importações com erro. Só as 20 últimas por família, até 30 dias.

use uuid::Uuid;

use crate::AppState;

/// Até 200 KB do texto colado.
const MAXIMO: usize = 200_000;

/// Guarda a importação que falhou (erros ao guardar só vão para o log: não atrapalham a resposta).
pub async fn falhou(st: &AppState, familia: Uuid, historia: Option<Uuid>, tipo: &str, texto: &str, erro: &str) {
    let mut fim = texto.len().min(MAXIMO);
    while !texto.is_char_boundary(fim) {
        fim -= 1;
    }
    let r = async {
        sqlx::query("INSERT INTO jobs.importacoes_erro (familia_id, historia_id, tipo, texto, erro) VALUES ($1, $2, $3, $4, $5)")
            .bind(familia)
            .bind(historia)
            .bind(tipo)
            .bind(&texto[..fim])
            .bind(erro)
            .execute(&st.db)
            .await?;
        sqlx::query(
            "DELETE FROM jobs.importacoes_erro WHERE criado_em < now() - interval '30 days'
                OR (familia_id = $1 AND id NOT IN (SELECT id FROM jobs.importacoes_erro WHERE familia_id = $1 ORDER BY id DESC LIMIT 20))",
        )
        .bind(familia)
        .execute(&st.db)
        .await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = r {
        tracing::warn!("não guardei a importação com erro: {e:#}");
    }
}

/// Uma importação com erro na tela do Sistema.
#[derive(sqlx::FromRow)]
pub struct Linha {
    pub id: i64,
    pub familia: String,
    pub historia: Option<String>,
    pub tipo: String,
    pub texto: String,
    pub erro: String,
    pub quando: String,
}

impl Linha {
    pub fn rotulo_tipo(&self) -> &'static str {
        match self.tipo.as_str() {
            "historia" => "📖 História nova",
            "ajuda" => "🤖 Ajuda da IA no editor",
            "memoria" => "🧠 Memória",
            _ => "?",
        }
    }
    pub fn tamanho(&self) -> String {
        let n = self.texto.len();
        if n >= 1024 { format!("{:.1} KB", n as f64 / 1024.0) } else { format!("{n} bytes") }
    }
}

/// As últimas importações com erro de todas as famílias (mais novas primeiro).
pub async fn ultimas(st: &AppState) -> Vec<Linha> {
    sqlx::query_as::<_, Linha>(
        "SELECT i.id, f.nome AS familia, h.titulo AS historia, i.tipo, i.texto, i.erro,
                to_char(i.criado_em AT TIME ZONE 'America/Sao_Paulo', 'DD/MM HH24:MI') AS quando
           FROM jobs.importacoes_erro i JOIN acesso.familias f ON f.id = i.familia_id
           LEFT JOIN conteudo.historias h ON h.id = i.historia_id
          ORDER BY i.id DESC LIMIT 100",
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default()
}
