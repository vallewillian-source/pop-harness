//! Tabela `matrix_midia` (MX-46): o que o espelho publicou na sala e quando expira.
//!
//! Uma linha por mídia publicada (`m.image`/`m.file` da MX-43). A rotina de expiração lê esta
//! tabela, apaga o arquivo no Synapse pela API admin e redige o evento na sala — e grava o carimbo
//! de cada etapa aqui, para repetir a passagem não repetir chamada.
//!
//! Regras que este repositório garante:
//! - **toda operação recebe a org**; a única leitura sem org seria a descoberta, que fica no
//!   repositório `matrix` (`orgs_ligadas`);
//! - **gravar a mesma mídia duas vezes não duplica**: `id` é o id do anexo e a segunda gravação não
//!   muda nada (o `txnId` do envio é derivado dele, então o Synapse devolve o mesmo evento);
//! - **cada etapa tem carimbo próprio**: `redacted_at` e `deleted_at` são gravados separados, e a
//!   fila só devolve a linha enquanto faltar uma das duas;
//! - **a espera da falha é do item**, não da rotina: `attempts` e `next_attempt_at` tiram o item
//!   problemático da frente em vez de repetir a chamada a cada passagem;
//! - **nada de dado de cliente**: nome do arquivo e ids já estão no evento da sala; aqui ficam o
//!   `mxc`, o tipo e o código curto do último erro, nada mais.

use poph_types::{AttachmentId, ConversationId, DomainError, OrgId};
use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::erro::{ErroStore, Resultado};
use crate::repos::{id_da_coluna, u32_da_coluna, u64_da_coluna};

/// Limite de caracteres do código de erro gravado (o mesmo `CHECK` da tabela).
pub const LIMITE_ERRO_MIDIA: usize = 64;

/// Limite de linhas que a fila da expiração devolve por org e por passagem.
pub const LIMITE_PENDENTES: u32 = 50;

/// Categoria da mídia, a mesma que a projeção do Python grava (`image`/`document`).
///
/// O frontend só separa imagem de arquivo (`kindFromPayload` em `expiredAttachmentMarker.ts`), mas
/// o valor gravado é o do Python, para as duas origens desenharem o mesmo aviso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoriaMidia {
    /// Imagem (`m.image`).
    Imagem,
    /// Documento e qualquer outro anexo (`m.file`).
    Documento,
}

impl CategoriaMidia {
    /// Texto gravado na coluna `kind`.
    pub fn como_texto(self) -> &'static str {
        match self {
            Self::Imagem => "image",
            Self::Documento => "document",
        }
    }

    /// Categoria a partir do texto gravado.
    pub fn de_texto(valor: &str) -> rusqlite::Result<Self> {
        match valor {
            "image" => Ok(Self::Imagem),
            "document" => Ok(Self::Documento),
            outro => Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!(
                    "categoria de mídia do Matrix desconhecida no banco: `{outro}`"
                ))),
            )),
        }
    }
}

/// Mídia que o espelho publicou numa sala, pronta para gravar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovaMidia {
    /// Id do anexo (`attachments.id`): é a chave da linha.
    pub id: AttachmentId,
    /// Conversa dona da sala.
    pub conversa: ConversationId,
    /// Agente da conversa: o fantasma dele redige o evento.
    pub agente: String,
    /// Sala em que o evento foi publicado.
    pub sala: String,
    /// Id do evento de mídia na sala.
    pub evento: String,
    /// `mxc://servidor/id` do arquivo que subiu.
    pub mxc: String,
    /// Nome do arquivo, como está no evento.
    pub nome: String,
    /// Tipo MIME real.
    pub mime: String,
    /// Categoria do anexo.
    pub categoria: CategoriaMidia,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// Quando o arquivo entrou no Synapse (ms). É daqui que o prazo conta.
    pub enviada_em: i64,
}

/// Uma mídia da sala com o que falta fazer nela (linha da fila da expiração).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiaDaSala {
    /// Id do anexo.
    pub id: AttachmentId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa dona da sala.
    pub conversa: ConversationId,
    /// Agente da conversa (escolhe o fantasma que redige).
    pub agente: String,
    /// Sala do evento.
    pub sala: String,
    /// Evento de mídia na sala.
    pub evento: String,
    /// `mxc://servidor/id` do arquivo.
    pub mxc: String,
    /// Nome do arquivo.
    pub nome: String,
    /// Tipo MIME.
    pub mime: String,
    /// Categoria do anexo.
    pub categoria: CategoriaMidia,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// Quando o arquivo entrou no Synapse (ms).
    pub enviada_em: i64,
    /// Quando o evento foi redigido (ms); `None` = falta redigir.
    pub redigida_em: Option<i64>,
    /// Quando o arquivo foi apagado do Synapse (ms); `None` = falta apagar.
    pub apagada_em: Option<i64>,
    /// Falhas seguidas da última etapa que falhou.
    pub tentativas: u32,
    /// Quando o item volta a ser tentado (ms).
    pub proxima_tentativa_em: i64,
    /// Código curto do último erro, quando houve.
    pub ultimo_erro: Option<String>,
}

impl MidiaDaSala {
    /// Falta redigir o evento na sala.
    pub fn falta_redigir(&self) -> bool {
        self.redigida_em.is_none()
    }

    /// Falta apagar o arquivo no Synapse.
    pub fn falta_apagar(&self) -> bool {
        self.apagada_em.is_none()
    }
}

/// Acesso à tabela `matrix_midia`.
pub struct MatrixMidia<'a>(pub(crate) &'a Store);

impl MatrixMidia<'_> {
    /// Grava a mídia publicada. Repetir a mesma mídia (mesmo id de anexo) não muda nada.
    ///
    /// Devolve `true` quando a linha é nova. A repetição é o caminho normal de uma reentrega do
    /// Synapse: o evento é o mesmo, e o prazo continua contando da primeira publicação.
    pub fn registrar(&self, org: OrgId, midia: &NovaMidia) -> Resultado<bool> {
        exigir_texto("mídia da sala", "sala", &midia.sala, 255)?;
        exigir_texto("mídia da sala", "evento", &midia.evento, 255)?;
        exigir_texto("mídia da sala", "agente", &midia.agente, 64)?;
        exigir_texto("mídia da sala", "mxc", &midia.mxc, 512)?;
        let agora = self.0.agora();
        let texto_org = org.to_string();
        let id = midia.id.to_string();
        let conversa = midia.conversa.to_string();
        let agente = midia.agente.clone();
        let sala = midia.sala.clone();
        let evento = midia.evento.clone();
        let mxc = midia.mxc.clone();
        let nome = midia.nome.clone();
        let mime = midia.mime.clone();
        let categoria = midia.categoria.como_texto();
        let tamanho = i64::try_from(midia.tamanho_bytes).unwrap_or(i64::MAX);
        let enviada_em = midia.enviada_em;
        self.0.escrita(move |conn| {
            let inseridas = conn
                .execute(
                    "INSERT INTO matrix_midia (
                         id, org_id, conversation_id, agent_id, room_id, event_id, mxc, name, mime,
                         kind, size_bytes, sent_at, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
                     ON CONFLICT (id) DO NOTHING",
                    params![
                        id, texto_org, conversa, agente, sala, evento, mxc, nome, mime, categoria,
                        tamanho, enviada_em, agora,
                    ],
                )
                .map_err(|erro| ErroStore::da_escrita("mídia da sala", erro))?;
            Ok(inseridas == 1)
        })
    }

    /// Mídias da org com alguma etapa pendente e vencidas em `corte` (ms), prontas para tentar em
    /// `agora` (ms), da mais antiga para a mais nova.
    ///
    /// `corte` é o instante limite da retenção: entra o que entrou no Synapse até ele. O item que
    /// falhou e está em espera (`next_attempt_at` no futuro) não volta antes da hora.
    pub fn pendentes(
        &self,
        org: OrgId,
        corte: i64,
        agora: i64,
        limite: u32,
    ) -> Resultado<Vec<MidiaDaSala>> {
        let limite = limite.clamp(1, LIMITE_PENDENTES) as i64;
        let texto_org = org.to_string();
        self.0.leitura(move |conn| {
            let mut consulta = conn.prepare(
                "SELECT id, org_id, conversation_id, agent_id, room_id, event_id, mxc, name, mime,
                        kind, size_bytes, sent_at, redacted_at, deleted_at, attempts,
                        next_attempt_at, last_error
                   FROM matrix_midia
                  WHERE org_id = ?1
                    AND (redacted_at IS NULL OR deleted_at IS NULL)
                    AND sent_at <= ?2
                    AND next_attempt_at <= ?3
                  ORDER BY sent_at, id
                  LIMIT ?4",
            )?;
            let linhas =
                consulta.query_map(params![texto_org, corte, agora, limite], linha_midia)?;
            let mut midias = Vec::new();
            for linha in linhas {
                midias.push(linha?);
            }
            Ok(midias)
        })
    }

    /// Marca o evento como redigido na sala e zera a espera (o item andou).
    pub fn marcar_redigida(&self, org: OrgId, id: AttachmentId, agora: i64) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET redacted_at = COALESCE(redacted_at, ?3),
                        attempts = 0,
                        next_attempt_at = 0,
                        last_error = NULL,
                        updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora],
            )
            .map(|_| ())
            .map_err(|erro| ErroStore::da_escrita("mídia da sala", erro))
        })
    }

    /// Marca o arquivo como apagado do Synapse e zera a espera (o item andou).
    pub fn marcar_apagada(&self, org: OrgId, id: AttachmentId, agora: i64) -> Resultado<()> {
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET deleted_at = COALESCE(deleted_at, ?3),
                        attempts = 0,
                        next_attempt_at = 0,
                        last_error = NULL,
                        updated_at = ?3
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, agora],
            )
            .map(|_| ())
            .map_err(|erro| ErroStore::da_escrita("mídia da sala", erro))
        })
    }

    /// Registra a falha de uma etapa: soma a tentativa, grava o código curto do erro e adia o item
    /// para `proxima_tentativa_em` (ms).
    ///
    /// É o que impede a rotina de repetir a mesma chamada a cada passagem: enquanto a linha estiver
    /// adiada, [`MatrixMidia::pendentes`] não a devolve.
    pub fn adiar(
        &self,
        org: OrgId,
        id: AttachmentId,
        proxima_tentativa_em: i64,
        codigo: &str,
        agora: i64,
    ) -> Resultado<u32> {
        let codigo: String = codigo.chars().take(LIMITE_ERRO_MIDIA).collect();
        let texto_org = org.to_string();
        let texto_id = id.to_string();
        self.0.escrita(move |conn| {
            conn.execute(
                "UPDATE matrix_midia
                    SET attempts = attempts + 1,
                        next_attempt_at = ?3,
                        last_error = ?4,
                        updated_at = ?5
                  WHERE org_id = ?1 AND id = ?2",
                params![texto_org, texto_id, proxima_tentativa_em, codigo, agora],
            )
            .map_err(|erro| ErroStore::da_escrita("mídia da sala", erro))?;
            let tentativas: i64 = conn
                .query_row(
                    "SELECT attempts FROM matrix_midia WHERE org_id = ?1 AND id = ?2",
                    params![texto_org, texto_id],
                    |linha| linha.get(0),
                )
                .optional()
                .map_err(ErroStore::from)?
                .unwrap_or(0);
            Ok(u32::try_from(tentativas).unwrap_or(u32::MAX))
        })
    }

    /// Quantas mídias da org ainda têm etapa pendente (o número que a métrica publica).
    pub fn pendentes_da_org(&self, org: OrgId) -> Resultado<u32> {
        let texto_org = org.to_string();
        self.0.leitura(move |conn| {
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM matrix_midia
                      WHERE org_id = ?1 AND (redacted_at IS NULL OR deleted_at IS NULL)",
                    params![texto_org],
                    |linha| linha.get(0),
                )
                .map_err(ErroStore::from)?;
            Ok(u32::try_from(total).unwrap_or(u32::MAX))
        })
    }
}

/// Uma linha da fila da expiração.
fn linha_midia(linha: &rusqlite::Row<'_>) -> rusqlite::Result<MidiaDaSala> {
    Ok(MidiaDaSala {
        id: id_da_coluna(linha, 0)?,
        org_id: id_da_coluna(linha, 1)?,
        conversa: id_da_coluna(linha, 2)?,
        agente: linha.get(3)?,
        sala: linha.get(4)?,
        evento: linha.get(5)?,
        mxc: linha.get(6)?,
        nome: linha.get(7)?,
        mime: linha.get(8)?,
        categoria: CategoriaMidia::de_texto(&linha.get::<_, String>(9)?)?,
        tamanho_bytes: u64_da_coluna(linha, 10)?,
        enviada_em: linha.get(11)?,
        redigida_em: linha.get(12)?,
        apagada_em: linha.get(13)?,
        tentativas: u32_da_coluna(linha, 14)?,
        proxima_tentativa_em: linha.get(15)?,
        ultimo_erro: linha.get(16)?,
    })
}

/// Texto obrigatório e com teto, como as outras colunas de texto do esquema.
fn exigir_texto(
    entidade: &'static str,
    campo: &'static str,
    valor: &str,
    teto: usize,
) -> Resultado<()> {
    if valor.trim().is_empty() {
        return Err(ErroStore::Dominio(DomainError::Validation {
            field: campo,
            reason: format!("{entidade} não pode ficar vazia"),
        }));
    }
    let tamanho = valor.chars().count();
    if tamanho > teto {
        return Err(ErroStore::Dominio(DomainError::Validation {
            field: campo,
            reason: format!("{entidade} passa de {teto} caracteres ({tamanho})"),
        }));
    }
    Ok(())
}
