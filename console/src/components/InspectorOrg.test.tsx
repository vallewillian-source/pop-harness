
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { InspectorOrg, type InspectorOrgProps } from "../components/InspectorOrg";
import type { Agente } from "../api/agents";
import type { Org } from "../api/client";
import { FakeServer } from "../testing/server";
import "../testing/setup";
import { setLocale } from "../i18n";

const ORG: Org = {
    id: "01a208f7-1111-7000-8000-000000000001",
    name: "Sessão Matrix",
    slug: "sessao-matrix",
    status: "active",
    version: 3,
    created_at: Date.UTC(2026, 10, 23, 15, 8),
    updated_at: Date.UTC(2026, 10, 23, 16, 6),
    config: { retention_days: 30 } as unknown as Org["config"],
};

const AGENT_MATRIX: Agente = {
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

function stateMatrix(extra: Partial<Record<string, unknown>> = {}) {
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

type Scenario = {
    agentes?: Agente[] | "erro";
    matrix?: ReturnType<typeof stateMatrix> | "erro";
};

function servidor({ agentes: agents = [AGENT_MATRIX], matrix = stateMatrix() }: Scenario = {}) {
    const fake = new FakeServer();
    fake.respond("GET", "/agents", () =>
        agents === "erro"
            ? { status: 500, body: { code: "internal", message: "falha ao listar agentes", details: null } }
            : { status: 200, body: { items: agents, next_cursor: null } },
    );
    fake.respond("GET", "/matrix", () =>
        matrix === "erro"
            ? { status: 500, body: { code: "internal", message: "falha ao ler o matrix", details: null } }
            : { status: 200, body: matrix },
    );
    return fake.install();
}

function build(props: Partial<InspectorOrgProps> = {}) {
    const onClose = vi.fn();
    const onAccess = vi.fn();
    const onEdit = vi.fn();
    const onExport = vi.fn();
    const onDelete = vi.fn();
    const utilitarios = render(
        <MemoryRouter>
            <InspectorOrg
                org={ORG}
                no={{ id: "local:18490", situacao: "local" }}
                onClose={onClose}
                onAccess={onAccess}
                onEdit={onEdit}
                onExport={onExport}
                onDelete={onDelete}
                {...props}
            />
        </MemoryRouter>,
    );
    const drawer = screen.getByRole("complementary", { name: "Detalhes de Sessão Matrix" });
    return { drawer, utilitarios, onClose, onAccess, onEdit, onExport, onDelete };
}

describe("InspetorOrg · cabeçalho e badges (prancha 01b)", () => {
    it("mostra o nome, o slug com a versão de concorrência e o botão de fechar", async () => {
        servidor();
        const { drawer } = build();

        expect(within(drawer).getByRole("heading", { name: "Sessão Matrix" })).toBeInTheDocument();
        expect(
            within(drawer).getByText("slug: sessao-matrix · versão v3 (ETag/If-Match)"),
        ).toBeInTheDocument();
        expect(within(drawer).getByRole("button", { name: "Fechar inspetor" })).toBeInTheDocument();

        expect(document.activeElement).toBe(drawer);
    });

    it("mostra a situação, o nó primário e o id abreviado com o id inteiro no title", async () => {
        servidor();
        const { drawer } = build();

        const situacao = within(drawer).getByText("Ativa");
        expect(situacao.closest(".poph-status")).toHaveClass("poph-status--active");

        expect(within(drawer).getByText("Nó primário")).toHaveAttribute(
            "title",
            "local:18490 (nó primário)",
        );

        const id = within(drawer).getByText("ID: 01a208f7…");
        expect(id).toHaveAttribute("title", ORG.id);
    });

    it("instalação de nó único (id vazio) aparece como 'nó local'", async () => {
        servidor();
        const { drawer } = build({ no: { id: "", situacao: "local" } });

        expect(within(drawer).getAllByText("nó local")).toHaveLength(2);
        expect(within(drawer).queryByTitle("nó local")).not.toBeInTheDocument();
    });

    it("sem o estado do cluster (token de org) a instância aparece como — com explicação", async () => {
        servidor();
        const { drawer } = build({ no: null });

        const value = within(drawer).getByTitle(
            "A resposta não traz o estado das instâncias (token de organização)",
        );
        expect(value).toHaveTextContent("—");
    });

    it("organização inativa mostra a pílula inativa", async () => {
        servidor();
        const { drawer } = build({ org: { ...ORG, status: "inactive" } });

        expect(within(drawer).getByText("Inativa").closest(".poph-status")).toHaveClass(
            "poph-status--inactive",
        );
    });
});

describe("InspetorOrg · propriedades", () => {
    it("mostra retenção, datas completas em pt-BR e a instância alocada", async () => {
        servidor();
        const { drawer } = build();

        expect(within(drawer).getByText("Retenção de histórico")).toBeInTheDocument();
        expect(within(drawer).getByText("30 dias")).toBeInTheDocument();
        expect(within(drawer).getByText("local:18490 (nó primário)")).toBeInTheDocument();
        expect(within(drawer).getByText(/^23\/11\/2026 às \d\d:06$/)).toBeInTheDocument();
        expect(within(drawer).getByText(/^23\/11\/2026 às \d\d:08$/)).toBeInTheDocument();
    });

    it("retenção ausente na config aparece como 'padrão'", async () => {
        servidor();
        const { drawer } = build({ org: { ...ORG, config: {} as unknown as Org["config"] } });

        expect(within(drawer).getByText("padrão")).toBeInTheDocument();
    });

    it("Espelho Matrix conectado mostra o estado real, com o detalhe no title", async () => {
        servidor({ matrix: stateMatrix() });
        const { drawer } = build();

        const value = await within(drawer).findByText("Conectado");
        expect(value).toHaveAttribute("title", "Nada pendente de espelhamento nesta organização.");
    });

    it("Espelho Matrix ligado e desconectado aparece como 'Desconectado'", async () => {
        servidor({ matrix: stateMatrix({ connected: false }) });
        const { drawer } = build();

        const value = await within(drawer).findByText("Desconectado");
        expect(value).toHaveAttribute("title", expect.stringContaining("Synapse"));
    });

    it("Espelho Matrix desligado na organização aparece como 'Desligado nesta organização'", async () => {
        servidor({ matrix: stateMatrix({ enabled: false }) });
        const { drawer } = build();

        expect(await within(drawer).findByText("Desligado nesta organização")).toBeInTheDocument();
    });

    it("módulo não configurado na instância aparece como 'Não configurado'", async () => {
        servidor({ matrix: stateMatrix({ connected: null, enabled: false }) });
        const { drawer } = build();

        expect(await within(drawer).findByText("Não configurado")).toBeInTheDocument();
    });

    it("falha na rota do Matrix vira 'indisponível' com o motivo no title, sem derrubar a gaveta", async () => {
        servidor({ matrix: "erro" });
        const { drawer } = build();

        const value = await within(drawer).findByText("indisponível");
        expect(value).toHaveAttribute("title", "falha ao ler o matrix");
        expect(within(drawer).getByText("30 dias")).toBeInTheDocument();
        expect(await within(drawer).findByText("Atendente Matrix")).toBeInTheDocument();
    });
});

describe("InspetorOrg · agentes vinculados", () => {
    it("mostra um card por agente, com modelo, situação e link para o agente", async () => {
        servidor();
        const { drawer } = build();

        expect(await within(drawer).findByText("Atendente Matrix")).toBeInTheDocument();
        expect(within(drawer).getByText("gemini-3.8-flash · Ativo")).toBeInTheDocument();
        expect(within(drawer).getByText("Agentes vinculados")).toHaveTextContent(
            "Agentes vinculados (1)",
        );
        expect(within(drawer).getByRole("link", { name: /Atendente Matrix/ })).toHaveAttribute(
            "href",
            `/orgs/${ORG.id}/agents/agt_matrix`,
        );
    });

    it("agente sem modelo cai no padrão do servidor, sem inventar nome de modelo", async () => {
        servidor({ agentes: [{ ...AGENT_MATRIX, model: undefined, status: "inactive" }] });
        const { drawer } = build();

        expect(
            await within(drawer).findByText("modelo padrão do servidor · Inativo"),
        ).toBeInTheDocument();
    });

    it("organização sem agente mostra o estado vazio honesto", async () => {
        servidor({ agentes: [] });
        const { drawer } = build();

        expect(
            await within(drawer).findByText("Nenhum agente cadastrado nesta organização."),
        ).toBeInTheDocument();
        expect(within(drawer).getByText("Agentes vinculados")).not.toHaveTextContent("(");
    });

    it("falha na listagem de agentes aparece só no bloco, sem derrubar a gaveta", async () => {
        servidor({ agentes: "erro" });
        const { drawer } = build();

        const notice = await within(drawer).findByText("não consegui carregar os agentes");
        expect(notice).toHaveAttribute("title", "falha ao listar agentes");
        expect(within(drawer).getByText("30 dias")).toBeInTheDocument();
        expect(await within(drawer).findByText("Conectado")).toBeInTheDocument();
    });

    it("com mais de oito agentes mostra oito cards e o link para ver todos", async () => {
        const muitos = Array.from({ length: 10 }, (_, index) => ({
            ...AGENT_MATRIX,
            id: `agt_${index}`,
            name: `Atendente ${index}`,
            slug: `atendente-${index}`,
        }));
        servidor({ agentes: muitos });
        const { drawer } = build();

        expect(await within(drawer).findByText("Atendente 0")).toBeInTheDocument();
        expect(within(drawer).queryByText("Atendente 9")).not.toBeInTheDocument();
        expect(within(drawer).getByText("Ver todos os 10 agentes →")).toHaveAttribute(
            "href",
            `/orgs/${ORG.id}/agents`,
        );
    });

    it("organização de outro nó não navega para o agente", async () => {
        servidor();
        const { drawer } = build({
            no: { id: "node-2", situacao: "ok" },
            remote: true,
        });

        expect(await within(drawer).findByText("Atendente Matrix")).toBeInTheDocument();
        expect(within(drawer).queryByRole("link", { name: /Atendente Matrix/ })).not.toBeInTheDocument();
    });
});

describe("InspetorOrg · rodapé de ações", () => {
    it("com token global oferece Acessar, Editar, Exportar e Excluir, cada um chamando o pai", async () => {
        servidor();
        const user = userEvent.setup();
        const { drawer, onAccess, onEdit, onExport, onDelete } = build();

        await user.click(within(drawer).getByRole("button", { name: "Acessar Organização →" }));
        await user.click(within(drawer).getByRole("button", { name: "Editar" }));
        await user.click(within(drawer).getByRole("button", { name: "Exportar" }));
        await user.click(within(drawer).getByRole("button", { name: "Excluir Sessão Matrix" }));

        expect(onAccess).toHaveBeenCalledWith(ORG);
        expect(onEdit).toHaveBeenCalledWith(ORG);
        expect(onExport).toHaveBeenCalledWith(ORG);
        expect(onDelete).toHaveBeenCalledWith(ORG);
    });

    it("com token de organização o rodapé tem só o Acessar", async () => {
        servidor();
        const { drawer } = build({ canManage: false });

        expect(within(drawer).getByRole("button", { name: "Acessar Organização →" })).toBeInTheDocument();
        expect(within(drawer).queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(within(drawer).queryByRole("button", { name: "Exportar" })).not.toBeInTheDocument();
        expect(
            within(drawer).queryByRole("button", { name: "Excluir Sessão Matrix" }),
        ).not.toBeInTheDocument();
    });

    it("organização de outro nó desabilita Acessar e Editar, como na lista", async () => {
        servidor();
        const { drawer } = build({ no: { id: "node-2", situacao: "ok" }, remote: true });

        expect(within(drawer).getByRole("button", { name: "Acessar Organização →" })).toBeDisabled();
        expect(within(drawer).getByRole("button", { name: "Editar" })).toBeDisabled();
        expect(within(drawer).getByRole("button", { name: "Exportar" })).toBeEnabled();
    });
});

describe("InspetorOrg · comando de teste", () => {
    it("usa o slug do primeiro agente ativo e copia para o clipboard", async () => {
        servidor();
        const user = userEvent.setup();
        const escrever = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: escrever },
            configurable: true,
        });
        const { drawer } = build();

        const command = await within(drawer).findByText(
            "poph chat --org sessao-matrix atendente-matrix",
        );
        await user.click(within(drawer).getByRole("button", { name: "Copiar comando" }));

        expect(escrever).toHaveBeenCalledWith(command.textContent);
        expect(await within(drawer).findByRole("status")).toHaveTextContent("Copiado");
    });

    it("sem agente ativo o comando fica com o marcador <agente>", async () => {
        servidor({ agentes: [] });
        const { drawer } = build();

        expect(
            await within(drawer).findByText("poph chat --org sessao-matrix <agente>"),
        ).toBeInTheDocument();
    });
});

describe("InspetorOrg em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("mostra cabeçalho, propriedades, datas e agentes em inglês", async () => {
        servidor();
        setLocale("en-US");

        render(
            <MemoryRouter>
                <InspectorOrg
                    org={ORG}
                    no={{ id: "local:18490", situacao: "local" }}
                    onClose={vi.fn()}
                    onAccess={vi.fn()}
                />
            </MemoryRouter>,
        );

        const drawer = await screen.findByRole("complementary", { name: "Details of Sessão Matrix" });
        expect(within(drawer).getByText("slug: sessao-matrix · version v3 (ETag/If-Match)")).toBeInTheDocument();
        expect(within(drawer).getByRole("button", { name: "Close inspector" })).toBeInTheDocument();
        expect(within(drawer).getByText("Active")).toBeInTheDocument();
        expect(within(drawer).getByText("History retention")).toBeInTheDocument();
        expect(within(drawer).getByText("30 days")).toBeInTheDocument();
        expect(within(drawer).getByText("Primary node")).toBeInTheDocument();
        expect(within(drawer).getAllByText(/^11\/23\/2026 at \d\d:\d\d [AP]M$/)).toHaveLength(2);
        expect(within(drawer).getByText("Linked agents")).toHaveTextContent("Linked agents (1)");
        expect(await within(drawer).findByText("gemini-3.8-flash · Active")).toBeInTheDocument();
        expect(within(drawer).getByRole("button", { name: "Open Organization →" })).toBeInTheDocument();
    });

    it("troca o idioma da gaveta aberta sem recarregar", async () => {
        servidor();
        setLocale("pt-BR");
        const { drawer } = build();

        expect(within(drawer).getByText("Retenção de histórico")).toBeInTheDocument();

        setLocale("en-US");

        expect(await within(drawer).findByText("History retention")).toBeInTheDocument();
        expect(within(drawer).getByRole("button", { name: "Close inspector" })).toBeInTheDocument();
    });
});

describe("InspetorOrg · teclado", () => {
    it("Esc fecha a gaveta e delega o fechamento a quem a monta", async () => {
        servidor();
        const user = userEvent.setup();
        const { onClose } = build();

        await user.keyboard("{Escape}");

        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("Esc dentro de um diálogo aberto não fecha a gaveta por baixo", async () => {
        servidor();
        const { onClose } = build();

        const dialog = document.createElement("div");
        dialog.setAttribute("role", "dialog");
        document.body.appendChild(dialog);
        const evento = new KeyboardEvent("keydown", { key: "Escape", bubbles: true });
        evento.preventDefault();
        dialog.dispatchEvent(evento);

        await waitFor(() => expect(onClose).not.toHaveBeenCalled());
        dialog.remove();
    });
});
