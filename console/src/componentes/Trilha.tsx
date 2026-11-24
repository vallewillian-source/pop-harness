/**
 * Trilha de navegação da barra superior (UX_SPEC §2.2).
 *
 * Extraída de `Shell.tsx` na RL-01 sem mudança de markup: o caminho é traduzido em migalhas a
 * partir da URL e do nome das organizações da sessão.
 */

import { Link, useLocation } from "react-router-dom";
import { useNomeDoAgente } from "./nomeDoAgente";

export type Migalha = { rotulo: string; para?: string };

const ROTULOS_DAS_ABAS: Record<string, string> = {
    instructions: "Instruções",
    "knowledge-base": "Base de conhecimento",
    files: "Arquivos",
    routing: "Roteamento",
    integrations: "Integrações",
    "model-jev": "Modelo e Jev",
    "test-chat": "Chat de teste",
};

/**
 * `nomeAgente`: nome do agente da URL, quando já foi publicado (HR-49b). Sem ele, a migalha mostra
 * "Agente" no lugar do nome.
 */
export function trilha(
    caminho: string,
    orgs: { id: string; name: string }[],
    nomeAgente: string | null = null,
): Migalha[] {
    const partes = caminho.split("/").filter(Boolean);
    if (partes.length === 0) return [{ rotulo: "Console" }];
    if (partes[0] === "orgs") {
        const migalhas: Migalha[] = [{ rotulo: "Organizações", para: "/orgs" }];
        const id = partes[1];
        if (id) {
            const org = orgs.find((candidata) => candidata.id === id);
            migalhas.push({ rotulo: org?.name ?? id, para: `/orgs/${id}` });
            // Seções da organização (HR-40): agentes e as abas do agente.
            if (partes[2] === "agents") {
                migalhas.push({ rotulo: "Agentes", para: `/orgs/${id}/agents` });
                if (partes[3]) {
                    // Organizações / <Org> / Agentes / <Nome do agente> / <Aba>: o nome não é link,
                    // porque é o contexto atual.
                    const aba = partes[4];
                    const rotuloDaAba = aba ? ROTULOS_DAS_ABAS[aba] : undefined;
                    migalhas.push({ rotulo: nomeAgente ?? "Agente" });
                    if (rotuloDaAba) migalhas.push({ rotulo: rotuloDaAba });
                }
            } else if (partes[2] === "structure") {
                migalhas.push({ rotulo: "Estrutura" });
            } else if (partes[2] === "products") {
                migalhas.push({ rotulo: "Produtos" });
            } else if (partes[2] === "operacao") {
                migalhas.push({ rotulo: "Saúde da instância" });
            }
        }
        return migalhas;
    }
    return [{ rotulo: "Organizações", para: "/orgs" }, { rotulo: partes[0] ?? "" }];
}

/**
 * Trilha de navegação. Cada migalha é um passo com o seu próprio separador dentro, para que esconder
 * um passo não deixe um "/" órfão (CF-08).
 *
 * Em rotas com caminho longo (organização / agentes / agente / aba, cinco migalhas) o espaço acaba:
 * a migalha da tela atual nunca encolhe, e as anteriores encolhem por prioridade — quanto mais longe
 * da atual, mais cedo ela corta com reticências. O `flexShrink` de cada passo é essa distância, e o
 * texto completo de cada migalha cortada fica no `title`.
 */
export function Trilha({ orgs }: { orgs: { id: string; name: string }[] }) {
    const { pathname } = useLocation();
    const partes = pathname.split("/").filter(Boolean);
    const nomeAgente = useNomeDoAgente(partes[0] === "orgs" && partes[2] === "agents" ? partes[3] : undefined);
    const migalhas = trilha(pathname, orgs, nomeAgente);
    return (
        <nav className="poph-topbar__trilha" aria-label="Trilha de navegação">
            {migalhas.map((migalha, indice) => {
                const ultima = indice === migalhas.length - 1;
                // 0 na tela atual: ela mantém o tamanho do texto; 1 na anterior, e assim por diante.
                const distanciaDaAtual = migalhas.length - 1 - indice;
                return (
                    <span
                        key={`${migalha.rotulo}-${indice}`}
                        className={
                            ultima
                                ? "poph-topbar__passo poph-topbar__passo--atual"
                                : "poph-topbar__passo"
                        }
                        style={{ flexShrink: distanciaDaAtual }}
                    >
                        {indice > 0 && (
                            <span className="poph-topbar__separador" aria-hidden="true">
                                /
                            </span>
                        )}
                        {migalha.para && !ultima ? (
                            <Link to={migalha.para} title={migalha.rotulo}>
                                {migalha.rotulo}
                            </Link>
                        ) : (
                            <span
                                className={ultima ? "poph-topbar__atual" : "poph-topbar__migalha"}
                                title={ultima ? undefined : migalha.rotulo}
                                aria-current={ultima ? "page" : undefined}
                            >
                                {migalha.rotulo}
                            </span>
                        )}
                    </span>
                );
            })}
        </nav>
    );
}
