//! Fila de tarefas no Postgres (FOR UPDATE SKIP LOCKED) e o worker que a consome.

use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

use crate::AppState;

pub async fn enfileirar(st: &AppState, tipo: &str, familia_id: Uuid, payload: Value) -> anyhow::Result<i64> {
    let id = sqlx::query_scalar("INSERT INTO jobs.fila (tipo, familia_id, payload) VALUES ($1, $2, $3) RETURNING id")
        .bind(tipo)
        .bind(familia_id)
        .bind(payload)
        .fetch_one(&st.db)
        .await?;
    Ok(id)
}

#[derive(sqlx::FromRow)]
struct Job {
    id: i64,
    tipo: String,
    payload: Value,
}

/// Loop do worker: pega um job pendente por vez.
pub async fn worker(st: AppState) {
    // Jobs que estavam rodando quando o app caiu voltam para a fila.
    if let Err(e) = sqlx::query("UPDATE jobs.fila SET estado = 'pendente' WHERE estado = 'rodando'")
        .execute(&st.db)
        .await
    {
        tracing::error!("worker: falha ao recuperar jobs: {e}");
    }
    loop {
        match proximo(&st).await {
            Ok(Some(job)) => executar(&st, job).await,
            Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
            Err(e) => {
                tracing::error!("worker: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn proximo(st: &AppState) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as::<_, Job>(
        "UPDATE jobs.fila SET estado = 'rodando', tentativas = tentativas + 1, atualizado_em = now()
          WHERE id = (SELECT id FROM jobs.fila
                       WHERE estado = 'pendente' AND agendado_para <= now()
                       ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1)
          RETURNING id, tipo, payload",
    )
    .fetch_optional(&st.db)
    .await
}

async fn executar(st: &AppState, job: Job) {
    tracing::info!("job {} ({}) começou", job.id, job.tipo);
    let resultado = match job.tipo.as_str() {
        "gerar_historia" => crate::historias::executar_geracao(st, &job.payload).await.map(|_| None),
        "editar_historia" => crate::historias::executar_ajuda(st, &job.payload).await.map(Some),
        "gerar_narracao" => crate::narracao::executar_narracao(st, &job.payload).await.map(|_| None),
        "narrar_publico" => crate::narracao::executar_narracao_publica(st, &job.payload).await.map(|_| None),
        outro => Err(anyhow::anyhow!("tipo de job desconhecido: {outro}")),
    };
    let r = match resultado {
        Ok(saida) => {
            tracing::info!("job {} terminou", job.id);
            sqlx::query("UPDATE jobs.fila SET estado = 'feito', erro = NULL, resultado = $2, atualizado_em = now() WHERE id = $1")
                .bind(job.id)
                .bind(saida)
                .execute(&st.db)
                .await
        }
        Err(e) => {
            tracing::warn!("job {} falhou: {e:#}", job.id);
            sqlx::query("UPDATE jobs.fila SET estado = 'erro', erro = $2, atualizado_em = now() WHERE id = $1")
                .bind(job.id)
                .bind(format!("{e:#}"))
                .execute(&st.db)
                .await
        }
    };
    if let Err(e) = r {
        tracing::error!("worker: não consegui gravar o estado do job {}: {e}", job.id);
    }
}
