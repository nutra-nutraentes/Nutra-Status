//! Arquivos de dados em `data/` (branch `status-data`). Tudo aqui é público:
//! nunca gravar `Resultado::detalhe`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{EstadoServico, Incidente, Resultado, Status};

/// ~30 dias de amostras a cada 30 min.
pub const MAX_AMOSTRAS: usize = 1440;
pub const MAX_DIAS: usize = 90;
pub const MAX_INCIDENTES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Amostra {
    pub t: i64,
    pub s: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dia {
    pub dia: String,
    pub checks: u32,
    pub falhas: u32,
    #[serde(default)]
    pub degradados: u32,
    pub uptime_pct: f64,
}

#[derive(Serialize)]
struct Atual<'a> {
    atualizado_em: DateTime<Utc>,
    geral: Status,
    servicos: Vec<ServicoPublico<'a>>,
}

#[derive(Serialize)]
struct ServicoPublico<'a> {
    id: &'a str,
    nome: &'a str,
    grupo: &'a str,
    status: Status,
    latencia_ms: Option<u64>,
    desde: Option<DateTime<Utc>>,
}

pub struct Dados {
    dir: PathBuf,
    pub estado: BTreeMap<String, EstadoServico>,
    pub incidentes: Vec<Incidente>,
    pub diario: BTreeMap<String, Vec<Dia>>,
}

impl Dados {
    pub fn carregar(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir.join("history"))
            .with_context(|| format!("criando {}", dir.display()))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            estado: ler_ou_padrao(&dir.join("state.json"))?,
            incidentes: ler_ou_padrao(&dir.join("incidents.json"))?,
            diario: ler_ou_padrao(&dir.join("daily.json"))?,
        })
    }

    /// Grava histórico, agregado diário, incidentes, estado e current.json.
    pub fn salvar(&mut self, resultados: &[Resultado], agora: DateTime<Utc>) -> Result<()> {
        for r in resultados.iter().filter(|r| r.status != Status::Ignorado) {
            let caminho = self.dir.join("history").join(format!("{}.json", r.id));
            let mut amostras: Vec<Amostra> = ler_ou_padrao(&caminho)?;
            amostras.push(Amostra {
                t: agora.timestamp(),
                s: r.status,
                ms: r.latencia_ms,
            });
            cortar_inicio(&mut amostras, MAX_AMOSTRAS);
            gravar_linhas(&caminho, &amostras)?;
        }

        agregar_dia(&mut self.diario, resultados, agora);
        cortar_inicio(&mut self.incidentes, MAX_INCIDENTES);

        let servicos = resultados
            .iter()
            .map(|r| ServicoPublico {
                id: &r.id,
                nome: &r.nome,
                grupo: &r.grupo,
                status: r.status,
                latencia_ms: r.latencia_ms,
                desde: self.estado.get(&r.id).map(|e| e.desde),
            })
            .collect();
        let atual = Atual {
            atualizado_em: agora,
            geral: status_geral(resultados),
            servicos,
        };

        gravar(&self.dir.join("current.json"), &atual)?;
        gravar(&self.dir.join("daily.json"), &self.diario)?;
        gravar(&self.dir.join("incidents.json"), &self.incidentes)?;
        gravar(&self.dir.join("state.json"), &self.estado)?;
        Ok(())
    }
}

/// Pior status entre as checagens executadas.
pub fn status_geral(resultados: &[Resultado]) -> Status {
    resultados
        .iter()
        .map(|r| r.status)
        .filter(|s| *s != Status::Ignorado)
        .max_by_key(|s| s.gravidade())
        .unwrap_or(Status::Online)
}

/// Soma a rodada no dia corrente (UTC) de cada serviço e mantém só os últimos `MAX_DIAS`.
/// Degradado conta como disponível para o uptime, mas fica registrado à parte.
pub fn agregar_dia(
    diario: &mut BTreeMap<String, Vec<Dia>>,
    resultados: &[Resultado],
    agora: DateTime<Utc>,
) {
    let hoje = agora.format("%Y-%m-%d").to_string();
    for r in resultados.iter().filter(|r| r.status != Status::Ignorado) {
        let dias = diario.entry(r.id.clone()).or_default();
        if dias.last().map(|d| d.dia != hoje).unwrap_or(true) {
            dias.push(Dia {
                dia: hoje.clone(),
                checks: 0,
                falhas: 0,
                degradados: 0,
                uptime_pct: 100.0,
            });
        }
        let dia = dias.last_mut().expect("dia recém-inserido");
        dia.checks += 1;
        match r.status {
            Status::Offline => dia.falhas += 1,
            Status::Degradado => dia.degradados += 1,
            _ => {}
        }
        let pct = 100.0 * f64::from(dia.checks - dia.falhas) / f64::from(dia.checks);
        dia.uptime_pct = (pct * 100.0).round() / 100.0;
        cortar_inicio(dias, MAX_DIAS);
    }
}

fn cortar_inicio<T>(v: &mut Vec<T>, max: usize) {
    if v.len() > max {
        v.drain(..v.len() - max);
    }
}

fn ler_ou_padrao<T: DeserializeOwned + Default>(caminho: &Path) -> Result<T> {
    match fs::read_to_string(caminho) {
        Ok(texto) if !texto.trim().is_empty() => serde_json::from_str(&texto)
            .with_context(|| format!("JSON inválido em {}", caminho.display())),
        Ok(_) => Ok(T::default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("lendo {}", caminho.display())),
    }
}

fn gravar<T: Serialize + ?Sized>(caminho: &Path, valor: &T) -> Result<()> {
    let mut texto = serde_json::to_string_pretty(valor)?;
    texto.push('\n');
    fs::write(caminho, texto).with_context(|| format!("gravando {}", caminho.display()))
}

/// Um item por linha: diffs pequenos no git a cada rodada.
fn gravar_linhas<T: Serialize>(caminho: &Path, itens: &[T]) -> Result<()> {
    let mut texto = String::from("[\n");
    for (i, item) in itens.iter().enumerate() {
        texto.push_str(&serde_json::to_string(item)?);
        texto.push_str(if i + 1 < itens.len() { ",\n" } else { "\n" });
    }
    texto.push_str("]\n");
    fs::write(caminho, texto).with_context(|| format!("gravando {}", caminho.display()))
}

#[cfg(test)]
mod testes {
    use super::*;
    use chrono::TimeZone;

    fn res(id: &str, status: Status) -> Resultado {
        Resultado {
            id: id.into(),
            nome: id.into(),
            grupo: "G".into(),
            status,
            latencia_ms: Some(10),
            detalhe: Some("segredo".into()),
        }
    }

    #[test]
    fn agrega_e_vira_o_dia() {
        let mut diario = BTreeMap::new();
        let d1 = Utc.with_ymd_and_hms(2026, 10, 14, 23, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2026, 10, 15, 0, 30, 0).unwrap();
        agregar_dia(&mut diario, &[res("a", Status::Online)], d1);
        agregar_dia(&mut diario, &[res("a", Status::Offline)], d1);
        agregar_dia(&mut diario, &[res("a", Status::Degradado)], d1);
        agregar_dia(&mut diario, &[res("a", Status::Ignorado)], d1);
        agregar_dia(&mut diario, &[res("a", Status::Online)], d2);
        let dias = &diario["a"];
        assert_eq!(dias.len(), 2);
        assert_eq!(dias[0].checks, 3);
        assert_eq!(dias[0].falhas, 1);
        assert_eq!(dias[0].degradados, 1);
        assert_eq!(dias[0].uptime_pct, 66.67);
        assert_eq!(dias[1].dia, "2026-10-15");
        assert_eq!(dias[1].uptime_pct, 100.0);
    }

    #[test]
    fn corta_historico() {
        let mut v: Vec<u32> = (0..10).collect();
        cortar_inicio(&mut v, 4);
        assert_eq!(v, vec![6, 7, 8, 9]);
    }

    #[test]
    fn geral_ignora_ignorado() {
        assert_eq!(status_geral(&[res("a", Status::Ignorado)]), Status::Online);
        assert_eq!(
            status_geral(&[res("a", Status::Online), res("b", Status::Degradado)]),
            Status::Degradado
        );
    }

    #[test]
    fn arquivos_nao_vazam_detalhe() {
        let dir = std::env::temp_dir().join(format!("nutra-status-teste-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut dados = Dados::carregar(&dir).unwrap();
        let agora = Utc::now();
        dados
            .salvar(
                &[res("a", Status::Offline), res("b", Status::Online)],
                agora,
            )
            .unwrap();
        for entrada in walk(&dir) {
            let texto = fs::read_to_string(&entrada).unwrap();
            assert!(
                !texto.contains("segredo"),
                "{} vazou detalhe",
                entrada.display()
            );
        }
        let hist: Vec<Amostra> =
            serde_json::from_str(&fs::read_to_string(dir.join("history/a.json")).unwrap()).unwrap();
        assert_eq!(hist.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut saida = Vec::new();
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                saida.extend(walk(&p));
            } else {
                saida.push(p);
            }
        }
        saida
    }
}
