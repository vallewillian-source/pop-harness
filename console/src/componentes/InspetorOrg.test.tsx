/**
 * Testes do inspetor lateral no desenho da prancha 01b (CF-05).
 *
 * O que importa aqui: cada peça do desenho aparece com o dado real da API — badges (situação, nó,
 * id), propriedades com o estado do Matrix vindo de `GET …/matrix`, cards dos agentes da consulta
 * única de `GET …/agents` e o rodapé de ações —, e nenhum dos dois recursos pode deixar a gaveta
 * em "carregando…" para sempre: cada um termina em pronto ou erro, sozinho.
 *
 * A gaveta é montada direto (sem a lista) e o servidor falso responde às duas rotas.
 */

import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { InspetorOrg, type InspetorOrgProps } from "./InspetorOrg";
import type { Agente } from "../api/agentes";
import type { Org } from "../api/cliente";
import { ServidorFalso } from "../testes/servidor";
import "../testes/preparo";

const ORG: Org = {
    id: "01a208f7-1111-7000-8000-000000000001",
    name: "Sessão Matrix",
    slug: "sessao-matrix",
    status: "active",
    version: 3,
    // 23/11/2026 12:08 e 13:06 em UTC-3.
    created_at: Date.UTC(2026, 10, 23, 15, 8),
    updated_at: Date.UTC(2026, 10, 23, 16, 6),
    config: { retention_days: 30 } as unknown as Org["config"],
};

const AGENTE_MATRIX: Agente = {
    id: "agt_matrix",
    org_id: ORG.id,
    name: "Atendente Matrix",
    slug: "atendente-matrix",
    description: "",
    status: "active",
    provider: "gemini",
    model: "gemini-3.8-flash",
    default_department_id: null,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

/** Estado do módulo Matrix como a rota devolve (todos os campos do contrato). */
function estadoMatrix(extra: Partial<Record<string, unknown>> = {}) {
    return {
        org_id: ORG.id,
        enabled: true,
        connected: true,
        enabled_at: 1_700_000_000_000,
        last_failure: null,
        pending_failures: 0,
        backlog: { messages: 0, oldest_seconds: 0 },
        updated_at: 1_700_000_000_000,
        ...extra,
    };
}

type Cenario = {
    agentes?: Agente[] | "erro";
    matrix?: ReturnType<typeof estadoMatrix> | "erro";
};

/** Servidor falso com as duas rotas da gaveta (a mais específica primeiro). */
function servidor({ agentes = [AGENTE_MATRIX], matrix = estadoMatrix() }: Cenario = {}) {
    const falso = new ServidorFalso();
    falso.responder("GET", "/agents", () =>
        agentes === "erro"
            ? { status: 500, corpo: { code: "internal", message: "falha ao listar agentes", details: null } }
            : { status: 200, corpo: { items: agentes, next_cursor: null } },
    );
    falso.responder("GET", "/matrix", () =>
        matrix === "erro"
            ? { status: 500, corpo: { code: "internal", message: "falha ao ler o matrix", details: null } }
            : { status: 200, corpo: matrix },
    );
    return falso.instalar();
}

function montar(props: Partial<InspetorOrgProps> = {}) {
    const aoFechar = vi.fn();
    const aoAcessar = vi.fn();
    const aoEditar = vi.fn();
    const aoExportar = vi.fn();
    const aoExcluir = vi.fn();
    const utilitarios = render(
        <MemoryRouter>
            <InspetorOrg
                org={ORG}
                no={{ id: "local:18490", situacao: "local" }}
                aoFechar={aoFechar}
                aoAcessar={aoAcessar}
                aoEditar={aoEditar}
                aoExportar={aoExportar}
                aoExcluir={aoExcluir}
                {...props}
            />
        </MemoryRouter>,
    );
    const gaveta = screen.getByRole("complementary", { name: "Detalhes de Sessão Matrix" });
    return { gaveta, utilitarios, aoFechar, aoAcessar, aoEditar, aoExportar, aoExcluir };
}

describe("InspetorOrg · cabeçalho e badges (prancha 01b)", () => {
    it("mostra o nome, o slug com a versão de concorrência e o botão de fechar", async () => {
        servidor();
        const { gaveta } = montar();

        expect(within(gaveta).getByRole("heading", { name: "Sessão Matrix" })).toBeInTheDocument();
        expect(
            within(gaveta).getByText("slug: sessao-matrix · versão v3 (ETag/If-Match)"),
        ).toBeInTheDocument();
        expect(within(gaveta).getByRole("button", { name: "Fechar inspetor" })).toBeInTheDocument();

        // A gaveta recebe o foco ao abrir.
        expect(document.activeElement).toBe(gaveta);
    });

    it("mostra a situação, o nó primário e o id abreviado com o id inteiro no title", async () => {
        servidor();
        const { gaveta } = montar();

        const situacao = within(gaveta).getByText("Ativa");
        expect(situacao.closest(".poph-status")).toHaveClass("poph-status--ativo");

        expect(within(gaveta).getByText("Nó primário")).toHaveAttribute(
            "title",
            "local:18490 (nó primário)",
        );

        const id = within(gaveta).getByText("ID: 01a208f7…");
        expect(id).toHaveAttribute("title", ORG.id);
    });

    it("instalação de nó único (id vazio) aparece como 'nó local'", async () => {
        servidor();
        const { gaveta } = montar({ no: { id: "", situacao: "local" } });

        // O chip e a propriedade "Instância alocada" dizem a mesma coisa, sem tooltip redundante.
        expect(within(gaveta).getAllByText("nó local")).toHaveLength(2);
        expect(within(gaveta).queryByTitle("nó local")).not.toBeInTheDocument();
    });

    it("sem o estado do cluster (token de org) a instância aparece como — com explicação", async () => {
        servidor();
        const { gaveta } = montar({ no: null });

        const valor = within(gaveta).getByTitle(
            "A resposta não traz o estado das instâncias (token de organização)",
        );
        expect(valor).toHaveTextContent("—");
    });

    it("organização inativa mostra a pílula inativa", async () => {
        servidor();
        const { gaveta } = montar({ org: { ...ORG, status: "inactive" } });

        expect(within(gaveta).getByText("Inativa").closest(".poph-status")).toHaveClass(
            "poph-status--inativo",
        );
    });
});

describe("InspetorOrg · propriedades", () => {
    it("mostra retenção, datas completas em pt-BR e a instância alocada", async () => {
        servidor();
        const { gaveta } = montar();

        expect(within(gaveta).getByText("Retenção de histórico")).toBeInTheDocument();
        expect(within(gaveta).getByText("30 dias")).toBeInTheDocument();
        expect(within(gaveta).getByText("local:18490 (nó primário)")).toBeInTheDocument();
        expect(within(gaveta).getByText(/^23\/11\/2026 às \d\d:06$/)).toBeInTheDocument();
        expect(within(gaveta).getByText(/^23\/11\/2026 às \d\d:08$/)).toBeInTheDocument();
    });

    it("retenção ausente na config aparece como 'padrão'", async () => {
        servidor();
        const { gaveta } = montar({ org: { ...ORG, config: {} as unknown as Org["config"] } });

        expect(within(gaveta).getByText("padrão")).toBeInTheDocument();
    });

    it("Espelho Matrix conectado mostra o estado real, com o detalhe no title", async () => {
        servidor({ matrix: estadoMatrix() });
        const { gaveta } = montar();

        const valor = await within(gaveta).findByText("Conectado");
        expect(valor).toHaveAttribute("title", "Nada pendente de espelhamento nesta organização.");
    });

    it("Espelho Matrix ligado e desconectado aparece como 'Desconectado'", async () => {
        servidor({ matrix: estadoMatrix({ connected: false }) });
        const { gaveta } = montar();

        const valor = await within(gaveta).findByText("Desconectado");
        expect(valor).toHaveAttribute("title", expect.stringContaining("Synapse"));
    });

    it("Espelho Matrix desligado na organização aparece como 'Desligado nesta organização'", async () => {
        servidor({ matrix: estadoMatrix({ enabled: false }) });
        const { gaveta } = montar();

        expect(await within(gaveta).findByText("Desligado nesta organização")).toBeInTheDocument();
    });

    it("módulo não configurado na instância aparece como 'Não configurado'", async () => {
        servidor({ matrix: estadoMatrix({ connected: null, enabled: false }) });
        const { gaveta } = montar();

        expect(await within(gaveta).findByText("Não configurado")).toBeInTheDocument();
    });

    it("falha na rota do Matrix vira 'indisponível' com o motivo no title, sem derrubar a gaveta", async () => {
        servidor({ matrix: "erro" });
        const { gaveta } = montar();

        const valor = await within(gaveta).findByText("indisponível");
        expect(valor).toHaveAttribute("title", "falha ao ler o matrix");
        // O resto da gaveta continua inteiro.
        expect(within(gaveta).getByText("30 dias")).toBeInTheDocument();
        expect(await within(gaveta).findByText("Atendente Matrix")).toBeInTheDocument();
    });
});

describe("InspetorOrg · agentes vinculados", () => {
    it("mostra um card por agente, com modelo, situação e link para o agente", async () => {
        servidor();
        const { gaveta } = montar();

        expect(await within(gaveta).findByText("Atendente Matrix")).toBeInTheDocument();
        expect(within(gaveta).getByText("gemini-3.8-flash · Ativo")).toBeInTheDocument();
        expect(within(gaveta).getByText("Agentes vinculados")).toHaveTextContent(
            "Agentes vinculados (1)",
        );
        expect(within(gaveta).getByRole("link", { name: /Atendente Matrix/ })).toHaveAttribute(
            "href",
            `/orgs/${ORG.id}/agents/agt_matrix`,
        );
    });

    it("agente sem modelo cai no padrão do servidor, sem inventar nome de modelo", async () => {
        servidor({ agentes: [{ ...AGENTE_MATRIX, model: undefined, status: "inactive" }] });
        const { gaveta } = montar();

        expect(
            await within(gaveta).findByText("modelo padrão do servidor · Inativo"),
        ).toBeInTheDocument();
    });

    it("organização sem agente mostra o estado vazio honesto", async () => {
        servidor({ agentes: [] });
        const { gaveta } = montar();

        expect(
            await within(gaveta).findByText("Nenhum agente cadastrado nesta organização."),
        ).toBeInTheDocument();
        expect(within(gaveta).getByText("Agentes vinculados")).not.toHaveTextContent("(");
    });

    it("falha na listagem de agentes aparece só no bloco, sem derrubar a gaveta", async () => {
        servidor({ agentes: "erro" });
        const { gaveta } = montar();

        const aviso = await within(gaveta).findByText("não consegui carregar os agentes");
        expect(aviso).toHaveAttribute("title", "falha ao listar agentes");
        expect(within(gaveta).getByText("30 dias")).toBeInTheDocument();
        expect(await within(gaveta).findByText("Conectado")).toBeInTheDocument();
    });

    it("com mais de oito agentes mostra oito cards e o link para ver todos", async () => {
        const muitos = Array.from({ length: 10 }, (_, indice) => ({
            ...AGENTE_MATRIX,
            id: `agt_${indice}`,
            name: `Atendente ${indice}`,
            slug: `atendente-${indice}`,
        }));
        servidor({ agentes: muitos });
        const { gaveta } = montar();

        expect(await within(gaveta).findByText("Atendente 0")).toBeInTheDocument();
        expect(within(gaveta).queryByText("Atendente 9")).not.toBeInTheDocument();
        expect(within(gaveta).getByText("Ver todos os 10 agentes →")).toHaveAttribute(
            "href",
            `/orgs/${ORG.id}/agents`,
        );
    });

    it("organização de outro nó não navega para o agente", async () => {
        servidor();
        const { gaveta } = montar({
            no: { id: "node-2", situacao: "ok" },
            remota: true,
        });

        expect(await within(gaveta).findByText("Atendente Matrix")).toBeInTheDocument();
        expect(within(gaveta).queryByRole("link", { name: /Atendente Matrix/ })).not.toBeInTheDocument();
    });
});

describe("InspetorOrg · rodapé de ações", () => {
    it("com token global oferece Acessar, Editar, Exportar e Excluir, cada um chamando o pai", async () => {
        servidor();
        const usuario = userEvent.setup();
        const { gaveta, aoAcessar, aoEditar, aoExportar, aoExcluir } = montar();

        await usuario.click(within(gaveta).getByRole("button", { name: "Acessar Organização →" }));
        await usuario.click(within(gaveta).getByRole("button", { name: "Editar" }));
        await usuario.click(within(gaveta).getByRole("button", { name: "Exportar" }));
        await usuario.click(within(gaveta).getByRole("button", { name: "Excluir Sessão Matrix" }));

        expect(aoAcessar).toHaveBeenCalledWith(ORG);
        expect(aoEditar).toHaveBeenCalledWith(ORG);
        expect(aoExportar).toHaveBeenCalledWith(ORG);
        expect(aoExcluir).toHaveBeenCalledWith(ORG);
    });

    it("com token de organização o rodapé tem só o Acessar", async () => {
        servidor();
        const { gaveta } = montar({ podeGerenciar: false });

        expect(within(gaveta).getByRole("button", { name: "Acessar Organização →" })).toBeInTheDocument();
        expect(within(gaveta).queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(within(gaveta).queryByRole("button", { name: "Exportar" })).not.toBeInTheDocument();
        expect(
            within(gaveta).queryByRole("button", { name: "Excluir Sessão Matrix" }),
        ).not.toBeInTheDocument();
    });

    it("organização de outro nó desabilita Acessar e Editar, como na lista", async () => {
        servidor();
        const { gaveta } = montar({ no: { id: "node-2", situacao: "ok" }, remota: true });

        expect(within(gaveta).getByRole("button", { name: "Acessar Organização →" })).toBeDisabled();
        expect(within(gaveta).getByRole("button", { name: "Editar" })).toBeDisabled();
        expect(within(gaveta).getByRole("button", { name: "Exportar" })).toBeEnabled();
    });
});

describe("InspetorOrg · comando de teste", () => {
    it("usa o slug do primeiro agente ativo e copia para o clipboard", async () => {
        servidor();
        const usuario = userEvent.setup();
        const escrever = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: escrever },
            configurable: true,
        });
        const { gaveta } = montar();

        const comando = await within(gaveta).findByText(
            "poph chat --org sessao-matrix atendente-matrix",
        );
        await usuario.click(within(gaveta).getByRole("button", { name: "Copiar comando" }));

        expect(escrever).toHaveBeenCalledWith(comando.textContent);
        expect(await within(gaveta).findByRole("status")).toHaveTextContent("Copiado");
    });

    it("sem agente ativo o comando fica com o marcador <agente>", async () => {
        servidor({ agentes: [] });
        const { gaveta } = montar();

        expect(
            await within(gaveta).findByText("poph chat --org sessao-matrix <agente>"),
        ).toBeInTheDocument();
    });
});

describe("InspetorOrg · teclado", () => {
    it("Esc fecha a gaveta e delega o fechamento a quem a monta", async () => {
        servidor();
        const usuario = userEvent.setup();
        const { aoFechar } = montar();

        await usuario.keyboard("{Escape}");

        expect(aoFechar).toHaveBeenCalledTimes(1);
    });

    it("Esc dentro de um diálogo aberto não fecha a gaveta por baixo", async () => {
        servidor();
        const { aoFechar } = montar();

        // Um diálogo aberto por cima (o de exclusão, por exemplo) consome o Esc primeiro.
        const dialogo = document.createElement("div");
        dialogo.setAttribute("role", "dialog");
        document.body.appendChild(dialogo);
        const evento = new KeyboardEvent("keydown", { key: "Escape", bubbles: true });
        evento.preventDefault();
        dialogo.dispatchEvent(evento);

        await waitFor(() => expect(aoFechar).not.toHaveBeenCalled());
        dialogo.remove();
    });
});
