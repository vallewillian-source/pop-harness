//! Supervisão de plugins de integração em processo externo (HR-25, PRD §8.3).
//!
//! O contrato — manifesto, evento de entrada, comandos de saída, estado e saúde — está em
//! [`poph_types::integracao`]. Aqui fica o **host** desse contrato para o plugin em processo
//! separado:
//!
//! - [`Supervisor`]: sobe o programa, faz o handshake, entrega os segredos **só** ali, escreve os
//!   comandos com prazo, reinicia com backoff e marca a integração como caída depois do teto de
//!   falhas. Um [`Supervisor`] por integração: um plugin que trava ou imprime lixo não alcança as
//!   outras.
//! - [`leitor`]: leitura de JSON-lines com teto de tamanho (linha acima do teto é descartada e
//!   contada, nunca acumulada em memória).
//! - [`ColetorEventos`]: [`Receptor`](poph_types::Receptor) que guarda o que o plugin entregou,
//!   usado pelo CI e por quem só quer observar.
//!
//! O que **não** entra aqui: a integração nativa do WhatsApp (HR-26) e o transporte da `outbox`
//! para o plugin — a ligação é da integração do servidor, que chama
//! [`Integration`](poph_types::Integration) como chama qualquer canal.

pub mod coletor;
pub mod leitor;
pub mod supervisor;

pub use coletor::ColetorEventos;
pub use leitor::{LeitorLinhas, Proxima};
pub use supervisor::{Espera, LimitesSupervisor, OpcoesPlugin, RelatoContadores, Supervisor};
