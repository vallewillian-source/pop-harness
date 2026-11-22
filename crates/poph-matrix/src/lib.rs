//! Módulo Matrix do POP Harness — MX-07 e MX-09.
//!
//! O módulo é **desligável** e vive só neste crate: `poph-types` não o conhece, `poph-agent` não o
//! importa (regra da tabela de crates do `AGENTS.md`), e as rotas HTTP entram em `poph-server` na
//! MX-16. Liga e desliga pela seção `[matrix]` do `poph.toml`; ausente ou `habilitado = false`,
//! nada do módulo é carregado — nem os tokens são lidos do ambiente.
//!
//! Entregue até aqui:
//!
//! - **MX-07** — a base: a configuração ([`config::MatrixConfig`], que mora em `poph-types`) e os
//!   segredos do Application Service, sempre no tipo `Sensivel`.
//! - **MX-09** — o cliente da API do Matrix ([`cliente::ClienteMatrix`]): salas, eventos, estado,
//!   convite, entrada, membros, perfil, registro de usuário virtual e `m.space.child`, com
//!   classificação de erro em transitório/definitivo ([`erro::ErroChamada`]).
//! - **MX-10** — os usuários virtuais ([`usuarios`]): montagem dos ids dos fantasmas com `localpart`
//!   válida e única, registro idempotente, nome de exibição atualizado só quando muda e cache em
//!   memória com teto.
//! - **MX-11** — a sala de cada conversa ([`salas`]): alias determinístico como trava da criação,
//!   tipo e metadata que o Element lê, fantasmas dentro da sala, estado `creating` → `ready` e
//!   aviso de que o histórico anterior não foi espelhado. A persistência entra pela porta
//!   [`salas::PortaSalas`], implementada por quem junta as peças.
//! - **MX-12** — os espaços por departamento ([`espacos`]): resolução do departamento (P3),
//!   criação do espaço pelo alias `#dep_<departamento>`, ligação a um espaço que já existe no
//!   Element (com erro claro quando falta permissão), sala dentro do espaço (`m.space.child`,
//!   `m.space.parent` e entrada restrita) e movimentação quando a transferência troca o
//!   departamento. A persistência entra pela porta [`espacos::PortaEspacos`].
//! - **MX-13** — o formato das mensagens espelhadas ([`formato`]).
//! - **MX-17** — o classificador dos eventos que chegam do Synapse ([`eventos`]): resposta do
//!   atendente, mídia do atendente ou ruído a ignorar, com motivo enumerado e anti-eco dos
//!   fantasmas desta instância.
//!
//! As tabelas, os laços e a API administrativa entram nas tarefas seguintes (MX-14 em diante).
//!
//! ```no_run
//! # fn exemplo() -> Result<(), Box<dyn std::error::Error>> {
//! use poph_matrix::cliente::ClienteMatrix;
//! use poph_types::MatrixConfig;
//!
//! // `None` quando a seção não existe ou está desligada: o módulo inteiro fica morto.
//! if let Some(modulo) = poph_matrix::config::ModuloMatrix::preparar(None)? {
//!     let cliente = ClienteMatrix::das_partes(&modulo.config, &modulo.segredos)?;
//!     let _url = cliente.url_base();
//!     // O `as_token` só sai de `Segredo::expor()`, dentro do cliente, para o cabeçalho.
//! }
//! # Ok(())
//! # }
//! ```

pub mod cliente;
pub mod config;
pub mod erro;
pub mod espacos;
pub mod eventos;
pub mod formato;
pub mod salas;
pub mod usuarios;

pub use cliente::{
    ClienteMatrix, ConfigCliente, CriarSala, MembroSala, Perfil, TEMPO_LIMITE_PADRAO,
};
pub use config::{
    ErroMatrix, ModuloMatrix, ResultadoMatrix, SegredosMatrix, VAR_ADMIN_TOKEN, VAR_AS_TOKEN,
    VAR_HS_TOKEN,
};
pub use erro::{Classificacao, ErroChamada};
pub use espacos::{
    DepartamentoEspaco, ErroEspaco, EspacoGravado, Espacos, LimitesConvite, OrigemEspaco,
    PortaEspacos, Posicao, SalaParaEspaco, TIPO_DEPARTAMENTO, TIPO_ESPACO, TIPO_FILHO, TIPO_PAI,
    TIPO_REGRAS_ENTRADA, alias_do_departamento, localpart_do_alias, resolver_departamento,
};
pub use eventos::{
    Acao, CAMPO_NOVO_CONTEUDO, MSGTYPE_AVISO, MSGTYPE_EMOTE, MotivoIgnorar, RELACAO_ANOTACAO,
    RELACAO_SUBSTITUICAO, TIPO_REACAO, TIPO_REMOCAO, TipoMidia, classificar,
};
pub use salas::{
    AVISO_SEM_HISTORICO, ConversaParaSala, ErroPorta, ErroSala, EstadoGravado, MetadataSala,
    PODER_MINIMO_AGENTE, PortaSalas, Provisionador, SalaGravada, SalaPronta, TIPO_METADATA,
    TIPO_NOME, TIPO_SALA,
};
pub use usuarios::{
    ErroFantasmas, Fantasma, IdsFantasmas, LIMITE_CORPO_LOCALPART, MARCA_AGENTE, MARCA_APARELHO,
    MARCA_CLIENTE, SUFIXO_BOT, SUFIXO_NOME_CLIENTE, TETO_PADRAO_CACHE, UsuariosVirtuais,
};
