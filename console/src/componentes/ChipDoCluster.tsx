/**
 * Chip de identidade do cluster no topo do rail (CF-02; de volta no CF-06 rodada 1).
 *
 * O desenho tem dois topos diferentes para o rail: a prancha 01 (visão global, URL `/orgs`) mostra
 * este chip — o cluster inteiro; as pranchas 02 e 05 (uma organização aberta) mostram o combobox
 * `OrgWorkspaceSwitcherCombobox` (`SeletorDeOrganizacao`). Cada tela tem o seu, nunca os dois.
 *
 * Medidas da prancha 01: 32 px de altura, borda de 1 px, ícone de servidor em cinza (o mesmo glifo do
 * desenho, que a SofaUI não tem), nome em 12,5 px Medium e o ponto de situação à direita. O ponto é
 * verde com a instância respondendo, vermelho quando ela responde degradada (503) ou não responde, e
 * neutro enquanto a saúde não chegou.
 */

import type { SofaIconProps } from "@sofacoding/sofa-ui";

/** Situação da instância, do jeito que o ponto do chip a mostra. */
export type SituacaoDoCluster = "ok" | "degradada" | "sem-conexao" | "verificando";

const ROTULOS: Record<SituacaoDoCluster, string> = {
    ok: "instância respondendo",
    degradada: "instância degradada",
    "sem-conexao": "sem conexão com a instância",
    verificando: "verificando a instância",
};

const CLASSES: Record<SituacaoDoCluster, string> = {
    ok: "poph-rail__cluster-ponto",
    degradada: "poph-rail__cluster-ponto poph-rail__cluster-ponto--alerta",
    "sem-conexao": "poph-rail__cluster-ponto poph-rail__cluster-ponto--alerta",
    verificando: "poph-rail__cluster-ponto poph-rail__cluster-ponto--neutro",
};

/** Glifo de servidor do desenho (duas gavetas empilhadas): a SofaUI não tem um equivalente. */
function IconeServidor(props: SofaIconProps) {
    return (
        <svg
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.25"
            strokeLinecap="round"
            strokeLinejoin="round"
            {...props}
        >
            <path d="M4 5h16a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z" />
            <path d="M4 14h16a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-3a1 1 0 0 1 1-1z" />
            <path d="M6 7.5h.01" />
            <path d="M6 16.5h.01" />
        </svg>
    );
}

export function ChipDoCluster({
    situacao,
    compacta = false,
}: {
    situacao: SituacaoDoCluster;
    /** Modo foco: o chip vira um tile de 32×32 com o ícone e o nome no `title`. */
    compacta?: boolean;
}) {
    const rotulo = `POP Harness Cluster · ${ROTULOS[situacao]}`;
    return (
        <div
            className={
                compacta
                    ? "poph-rail__cluster poph-rail__cluster--compacta"
                    : "poph-rail__cluster"
            }
            title={rotulo}
        >
            <IconeServidor className="poph-rail__cluster-icone" aria-hidden="true" />
            <span className="poph-rail__cluster-nome">POP Harness Cluster</span>
            <span className={CLASSES[situacao]} aria-hidden="true" />
            <span className="poph-rail__so-leitor">{rotulo}</span>
        </div>
    );
}
