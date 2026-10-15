use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Situação de um serviço em uma rodada de checagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Degradado,
    Offline,
    /// Checagem não executada (ex.: secret não configurado). Não conta para uptime nem alerta.
    Ignorado,
}

impl Status {
    pub fn com_problema(self) -> bool {
        matches!(self, Status::Degradado | Status::Offline)
    }

    /// Peso para escolher o pior estado (maior = pior).
    pub fn gravidade(self) -> u8 {
        match self {
            Status::Ignorado => 0,
            Status::Online => 1,
            Status::Degradado => 2,
            Status::Offline => 3,
        }
    }

    pub fn rotulo(self) -> &'static str {
        match self {
            Status::Online => "online",
            Status::Degradado => "degradado",
            Status::Offline => "offline",
            Status::Ignorado => "ignorado",
        }
    }
}

/// Resultado completo de uma checagem. `detalhe` pode conter dados sensíveis
/// (IPs, mensagens de erro) e nunca é gravado nos arquivos públicos.
#[derive(Debug, Clone)]
pub struct Resultado {
    pub id: String,
    pub nome: String,
    pub grupo: String,
    pub status: Status,
    pub latencia_ms: Option<u64>,
    pub detalhe: Option<String>,
}

/// Estado persistido de um serviço entre rodadas (state.json).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstadoServico {
    pub status: Status,
    pub desde: DateTime<Utc>,
    #[serde(default)]
    pub ultimo_aviso: Option<DateTime<Utc>>,
    #[serde(default)]
    pub issue: Option<u64>,
    #[serde(default)]
    pub incidente: Option<String>,
}

/// Queda registrada em incidents.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incidente {
    pub id: String,
    pub servico: String,
    pub nome: String,
    pub status: Status,
    pub inicio: DateTime<Utc>,
    #[serde(default)]
    pub fim: Option<DateTime<Utc>>,
    #[serde(default)]
    pub duracao_s: Option<i64>,
    #[serde(default)]
    pub issue: Option<u64>,
}
