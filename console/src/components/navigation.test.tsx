
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { NavItemEntry, Rail } from "../components/Rail";
import type { Agente } from "../api/agents";
import {
    ITEMS_NAVIGATION,
    ITEMS_COCKPIT_AGENT,
    type ContextNavigation,
    type ItemNavigation,
} from "../components/navigation";
import { GuardChangesProvider, useGuardChanges } from "../components/GuardChanges";
import { setLocale } from "../i18n";
import { SessionProvider } from "../session/Session";
import { FakeServer, apiError } from "../testing/server";
import AppSource from "../App.tsx?raw";

const stateHealth = vi.hoisted(() => ({
    current: {
        health: { version: "0.9.1" } as { version: string } | null,
        disconnected: false,
        degradada: false,
    },
}));
vi.mock("./health", () => ({ useHealth: () => stateHealth.current }));

const KEY_SESSION = "poph-console.sessao";
const CONTEXT_EMPTY: ContextNavigation = { orgId: null, agentId: null, totalOrgs: null };

function item(label: string): ItemNavigation {
    const finding = ITEMS_NAVIGATION.find((candidate) => candidate.caption === label);
    if (!finding) throw new Error(`item de navegação ausente: ${label}`);
    return finding;
}

function Probe() {
    const { pathname } = useLocation();
    return <span data-testid="path">{pathname}</span>;
}

function Dirty() {
    const { register } = useGuardChanges();
    useEffect(() => {
        register({ dirty: true, save: async () => true });
        return () => register(null);
    }, [register]);
    return null;
}

function prepareSession(
    scope: "global" | "org",
    orgs: { id: string; name: string; slug?: string; status?: string }[],
) {
    window.sessionStorage.setItem(
        KEY_SESSION,
        JSON.stringify({
            token: scope === "org" ? "poph_teste" : "pophg_teste",
            escopo: scope,
            orgs,
        }),
    );
}

function buildRail(
    path: string,
    options: {
        dirty?: boolean;
        onOpenShortcuts?: () => void;
        agents?: Agente[];
        compact?: boolean;
    } = {},
) {
    return render(
        <MemoryRouter initialEntries={[path]}>
            <SessionProvider>
                <GuardChangesProvider>
                    <Rail
                        onOpenShortcuts={options.onOpenShortcuts ?? (() => {})}
                        agents={options.agents}
                        compact={options.compact}
                    />
                    {options.dirty && <Dirty />}
                    <Probe />
                </GuardChangesProvider>
            </SessionProvider>
        </MemoryRouter>,
    );
}

const TWO_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "inactive" },
];

beforeEach(() => {
    window.sessionStorage.clear();
});

afterEach(() => {
    window.sessionStorage.clear();
    vi.unstubAllGlobals();
});

describe("lista de navegação", () => {
    it("tem os grupos e itens do PRD, na ordem do desenho", () => {
        expect(ITEMS_NAVIGATION.map((entry) => entry.caption)).toEqual([
            "Organizações",
            "Estrutura",
            "Produtos e catálogo",
            "Agentes",
            "Modelo e Jev",
            "Inspeção de turnos",
            "Chat de teste",
            "WhatsApp e canais",
            "Matrix (Element)",
            "Plugins e extensões",
            "Saúde da instância",
        ]);
    });

    it("não tem mais Cluster nem Importar / exportar", () => {
        const labels = ITEMS_NAVIGATION.map((entry) => entry.caption);
        expect(labels).not.toContain("Cluster");
        expect(labels).not.toContain("Importar / exportar");
    });

    it("agrupa os itens em TENANTS E RECURSOS, RUNTIME E MOTOR e SISTEMA", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        expect(screen.getByText("TENANTS E RECURSOS")).toBeInTheDocument();
        expect(screen.getByText("RUNTIME E MOTOR")).toBeInTheDocument();
        expect(screen.getByText("SISTEMA")).toBeInTheDocument();
        expect(screen.queryByText("Cluster")).toBeNull();
        expect(screen.queryByText("Importar / exportar")).toBeNull();
    });
});

describe("itens do menu", () => {
    it("desabilita item que exige organização quando não há organização na URL", () => {
        render(
            <MemoryRouter>
                <NavItemEntry item={item("Estrutura")} context={CONTEXT_EMPTY} />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Estrutura/ })).toBeNull();
        const button = screen.getByRole("button", { name: /Estrutura/ });
        expect(button).toBeDisabled();
        expect(button).not.toHaveTextContent("em breve");
    });

    it("vira link para a rota da organização quando ela está na URL", () => {
        const cases: [string, string][] = [
            ["Estrutura", "/orgs/org_1/structure"],
            ["Produtos e catálogo", "/orgs/org_1/products"],
            ["Agentes", "/orgs/org_1/agents"],
        ];
        for (const [label, route] of cases) {
            const rendered = render(
                <MemoryRouter>
                    <NavItemEntry
                        item={item(label)}
                        context={{ orgId: "org_1", agentId: null, totalOrgs: null }}
                    />
                </MemoryRouter>,
            );
            expect(screen.getByRole("link", { name: label })).toHaveAttribute("href", route);
            rendered.unmount();
        }
    });

    it("Modelo e Jev fica desabilitado sem agente na URL", () => {
        render(
            <MemoryRouter>
                <NavItemEntry
                    item={item("Modelo e Jev")}
                    context={{ orgId: "org_1", agentId: null, totalOrgs: null }}
                />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Modelo e Jev/ })).toBeNull();
        expect(screen.getByRole("button", { name: /Modelo e Jev/ })).toBeDisabled();
    });

    it("Modelo e Jev com agente na URL é link para a aba real do agente", () => {
        render(
            <MemoryRouter>
                <NavItemEntry
                    item={item("Modelo e Jev")}
                    context={{ orgId: "org_1", agentId: "agt_1", totalOrgs: null }}
                />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: "Modelo e Jev" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/model-jev",
        );
    });

    it("item sem exigência continua link sem organização aberta", () => {
        render(
            <MemoryRouter>
                <NavItemEntry item={item("Organizações")} context={CONTEXT_EMPTY} />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute("href", "/orgs");
    });

    it("item em breve não navega mesmo com organização aberta", async () => {
        const user = userEvent.setup();
        render(
            <MemoryRouter initialEntries={["/orgs/org_1"]}>
                <NavItemEntry
                    item={item("Plugins e extensões")}
                    context={{ orgId: "org_1", agentId: null, totalOrgs: null }}
                />
                <Probe />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Plugins e extensões/ })).toBeNull();
        const button = screen.getByRole("button", { name: /Plugins e extensões/ });
        expect(button).toBeDisabled();
        expect(button).toHaveTextContent("em breve");

        await user.click(button);
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1");
    });

    it("contador de Organizações mostra o total da sessão", () => {
        render(
            <MemoryRouter>
                <NavItemEntry
                    item={item("Organizações")}
                    context={{ orgId: null, agentId: null, totalOrgs: 7 }}
                />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveTextContent("7");
    });
});

describe("CN-02: agentes sob o item Agentes", () => {
    function agent(partial: Partial<Agente> & Pick<Agente, "id" | "name">): Agente {
        return {
            org_id: "org_1",
            slug: partial.id,
            description: "",
            status: "active",
            default_department_id: null,
            human_handoff_department_id: null,
            version: 1,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            ...partial,
        };
    }

    const AGENTS = [
        agent({ id: "agt_1", name: "Atendente", slug: "atendente", status: "active" }),
        agent({ id: "agt_2", name: "Patinhas", slug: "patinhas", status: "inactive" }),
        agent({ id: "agt_3", name: "Arquivado", slug: "arquivado", status: "archived" }),
    ];

    it("lista os agentes da organização logo abaixo de Agentes, com a bolinha do status", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { agents: AGENTS });

        expect(screen.getByRole("link", { name: /Atendente/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/instructions",
        );
        expect(screen.getByRole("link", { name: /Patinhas/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_2/instructions",
        );
        expect(screen.getByRole("link", { name: /Arquivado/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_3/instructions",
        );
    });

    it("a bolinha é verde no agente ativo e cinza nos demais, com o status no title", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { agents: AGENTS });

        expect(screen.getByTitle("Ativo")).toHaveClass("poph-rail__agent-dot--active");
        expect(screen.getByTitle("Inativo")).toHaveClass("poph-rail__agent-dot--idle");
        expect(screen.getByTitle("Arquivado")).toHaveClass("poph-rail__agent-dot--idle");
        expect(screen.getByLabelText("Ativo")).toBe(screen.getByTitle("Ativo"));
    });

    it("status desconhecido cai no cinza com o próprio texto", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", {
            agents: [agent({ id: "agt_4", name: "Novo", status: "paused" })],
        });

        expect(screen.getByTitle("paused")).toHaveClass("poph-rail__agent-dot--idle");
    });

    it("clicar no agente leva direto para as Instruções dele", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { agents: AGENTS });

        await user.click(screen.getByRole("link", { name: /Patinhas/ }));

        expect(screen.getByTestId("path")).toHaveTextContent(
            "/orgs/org_1/agents/agt_2/instructions",
        );
    });

    it("no modo foco o menu não lista os agentes", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { agents: AGENTS, compact: true });

        expect(screen.queryByRole("link", { name: /Atendente/ })).toBeNull();
        expect(screen.queryByTitle("Ativo")).toBeNull();
    });

    it("organização sem agentes mantém o item Agentes e não cria subitem", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { agents: [] });

        expect(screen.getByRole("link", { name: "Agentes" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
        expect(screen.queryByTitle("Ativo")).toBeNull();
    });

    it("sem organização aberta, o menu não lista agentes", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs", { agents: AGENTS });

        expect(screen.queryByRole("link", { name: /Atendente/ })).toBeNull();
    });
});

describe("Saúde da instância (HR-48a)", () => {
    it("com organização na URL, é link para a tela de operação, sem 'em breve'", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        const link = screen.getByRole("link", { name: /Saúde da instância/ });
        expect(link).toHaveAttribute("href", "/orgs/org_1/operacao");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("sem organização na URL, fica desabilitada com o motivo, sem 'em breve'", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        const button = screen.getByRole("button", { name: /Saúde da instância/ });
        expect(button).toBeDisabled();
        expect(button).not.toHaveTextContent("em breve");
    });
});

describe('"em breve" aparece só nos itens que ainda não têm tela', () => {
    it("com organização na URL, Estrutura, Produtos e Agentes são links sem 'em breve'", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        for (const label of ["Estrutura", "Produtos e catálogo", "Agentes"]) {
            const link = screen.getByRole("link", { name: label });
            expect(link).not.toHaveTextContent("em breve");
        }
    });

    it("com agente na URL, Chat de teste é link para a aba do agente, sem 'em breve'", () => {
        render(
            <MemoryRouter>
                <NavItemEntry
                    item={item("Chat de teste")}
                    context={{ orgId: "org_1", agentId: "agt_1", totalOrgs: null }}
                />
            </MemoryRouter>,
        );

        const link = screen.getByRole("link", { name: "Chat de teste" });
        expect(link).toHaveAttribute("href", "/orgs/org_1/agents/agt_1/test-chat");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("com agente na URL, Modelo e Jev é link sem 'em breve'", () => {
        render(
            <MemoryRouter>
                <NavItemEntry
                    item={item("Modelo e Jev")}
                    context={{ orgId: "org_1", agentId: "agt_1", totalOrgs: null }}
                />
            </MemoryRouter>,
        );

        const link = screen.getByRole("link", { name: "Modelo e Jev" });
        expect(link).toHaveAttribute("href", "/orgs/org_1/agents/agt_1/model-jev");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("sem organização na URL, os itens que dependem dela ficam desabilitados sem 'em breve' e com o motivo", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        for (const label of [
            "Estrutura",
            "Produtos e catálogo",
            "Agentes",
            "Modelo e Jev",
            "Chat de teste",
            "Saúde da instância",
            "Matrix (Element)",
        ]) {
            const button = screen.getByRole("button", { name: new RegExp(label.replace(/[()]/g, "\\$&")) });
            expect(button).toBeDisabled();
            expect(button).not.toHaveTextContent("em breve");
        }
        expect(screen.getAllByTitle("Requer uma organização aberta")).toHaveLength(7);
    });

    it("com organização mas sem agente, Modelo e Jev fica desabilitado com o motivo do agente", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        const button = screen.getByRole("button", { name: /Modelo e Jev/ });
        expect(button).toBeDisabled();
        expect(button).not.toHaveTextContent("em breve");
        const reasons = screen.getAllByTitle("Requer um agente aberto");
        expect(reasons.some((motivo) => motivo.contains(button))).toBe(true);
    });

    it("itens de estado emBreve continuam desabilitados e com 'em breve', mesmo com organização aberta", () => {
        for (const label of [
            "Inspeção de turnos",
            "WhatsApp e canais",
            "Plugins e extensões",
        ]) {
            const rendered = render(
                <MemoryRouter>
                    <NavItemEntry
                        item={item(label)}
                        context={{ orgId: "org_1", agentId: "agt_1", totalOrgs: null }}
                    />
                </MemoryRouter>,
            );
            const button = screen.getByRole("button", {
                name: new RegExp(label.replace(/[()]/g, "\\$&")),
            });
            expect(button).toBeDisabled();
            expect(button).toHaveTextContent("em breve");
            rendered.unmount();
        }
    });
});

describe("menu montado com a sessão", () => {
    it("sem org na URL, Estrutura, Produtos e catálogo, Agentes e Modelo e Jev ficam desabilitados", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        for (const label of ["Estrutura", "Produtos e catálogo", "Agentes", "Modelo e Jev", "Chat de teste"]) {
            expect(screen.getByRole("button", { name: new RegExp(label.replace(/[()]/g, "\\$&")) })).toBeDisabled();
        }
    });

    it("com org na URL, os links de organização apontam para a rota certa", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        expect(screen.getByRole("link", { name: "Estrutura" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );
        expect(screen.getByRole("link", { name: "Produtos e catálogo" })).toHaveAttribute(
            "href",
            "/orgs/org_1/products",
        );
        expect(screen.getByRole("link", { name: "Agentes" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
    });

    it("com agente na URL, o rail é o cockpit do agente, e não os grupos de organização", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Modelo & Parâmetros (Jev)" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/model-jev",
        );
        expect(screen.queryByText("TENANTS E RECURSOS")).toBeNull();
    });

    it("com organização aberta o menu não tem Organizações e o item ativo é o da tela", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        expect(screen.getByRole("link", { name: "Estrutura" })).toHaveAttribute(
            "aria-current",
            "page",
        );
        expect(screen.queryByRole("link", { name: /^Organizações/ })).toBeNull();
    });

    it("marca Organizações quando a tela é a lista", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute(
            "aria-current",
            "page",
        );
    });

    it("mostra o total de organizações da sessão no contador", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveTextContent("2");
    });

    it("o seletor mostra a organização aberta e lista as que o token vê, com Administrar Organizações", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        const selector = screen.getByRole("combobox", { name: "Organização: Loja Antiga" });
        expect(selector).toHaveTextContent("Loja Antiga");
        expect(screen.queryByRole("listbox")).toBeNull();

        await user.click(selector);

        expect(screen.getByRole("listbox", { name: "Organizações disponíveis" })).toBeInTheDocument();
        expect(
            screen.getAllByRole("option")[0],
        ).toHaveAccessibleName(/Administrar Organizações/);
        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toHaveAttribute(
            "aria-current",
            "true",
        );
        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toBeInTheDocument();
    });

    it("na lista de organizações, o topo é o chip do cluster — e não o combobox", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        const chip = screen.getByText("POP Harness Cluster");
        expect(chip).toBeInTheDocument();
        expect(chip.closest(".poph-rail__cluster")).toHaveAttribute(
            "title",
            "POP Harness Cluster · instância respondendo",
        );
        expect(screen.queryByRole("combobox")).toBeNull();
    });

    it("com organização aberta, o topo volta a ser o combobox do seletor", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        expect(screen.getByRole("combobox", { name: "Organização: Loja Antiga" })).toBeInTheDocument();
        expect(screen.queryByText("POP Harness Cluster")).toBeNull();
    });

    it("o ponto do chip acompanha a saúde: verde respondendo, vermelho degradada ou sem conexão", () => {
        prepareSession("global", TWO_ORGS);

        const point = () => {
            const { container, unmount } = buildRail("/orgs");
            const finding = container.querySelector(".poph-rail__cluster-point");
            const cssClass = finding?.className ?? "";
            unmount();
            return { cssClass, titulo: finding?.closest(".poph-rail__cluster")?.getAttribute("title") };
        };

        stateHealth.current = { health: { version: "0.9.1" }, disconnected: false, degradada: false };
        expect(point()).toEqual({
            cssClass: "poph-rail__cluster-point",
            titulo: "POP Harness Cluster · instância respondendo",
        });

        stateHealth.current = { health: null, disconnected: false, degradada: true };
        expect(point().titulo).toBe("POP Harness Cluster · instância degradada");
        expect(point().cssClass).toContain("poph-rail__cluster-point--alert");

        stateHealth.current = { health: null, disconnected: true, degradada: false };
        expect(point().titulo).toBe("POP Harness Cluster · sem conexão com a instância");
        expect(point().cssClass).toContain("poph-rail__cluster-point--alert");

        stateHealth.current = { health: null, disconnected: false, degradada: false };
        expect(point().titulo).toBe("POP Harness Cluster · verificando a instância");
        expect(point().cssClass).toContain("poph-rail__cluster-point--neutral");

        stateHealth.current = { health: { version: "0.9.1" }, disconnected: false, degradada: false };
    });

    it("fora das telas de organização, sem organização aberta, diz Selecionar organização", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/");

        expect(screen.getByRole("combobox")).toHaveTextContent("Selecionar organização");
    });

    it("sem nenhuma organização, o gatilho diz Nenhuma organização e o popover explica", async () => {
        const user = userEvent.setup();
        prepareSession("global", []);
        buildRail("/orgs/org_1/structure");

        const selector = screen.getByRole("combobox");
        expect(selector).toHaveTextContent("Nenhuma organização");

        await user.click(selector);

        expect(screen.queryByRole("option")).toBeNull();
        expect(screen.getByRole("status")).toHaveTextContent("Nenhuma organização");
    });

    it("com token de org, o gatilho mostra a própria organização e não abre a lista", async () => {
        const user = userEvent.setup();
        prepareSession("org", [{ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" }]);
        buildRail("/orgs/org_1/structure");

        const selector = screen.getByRole("combobox");
        expect(selector).toBeDisabled();
        expect(selector).toHaveTextContent("Loja Antiga");

        await user.click(selector);
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("escolher outra organização sem alterações pendentes navega para ela", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        await user.click(screen.getByRole("option", { name: /Clínica Nova/ }));

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_2");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("o primeiro item do combobox é Administrar Organizações e leva para a lista", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        expect(screen.getAllByRole("option")[0]).toBe(administrar);

        await user.click(administrar);

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("Administrar Organizações com alterações pendentes pede confirmação antes de sair", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { dirty: true });

        await user.click(screen.getByRole("combobox"));
        await user.click(screen.getByRole("option", { name: /Administrar Organizações/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("escolher a organização que já está aberta não navega", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        await user.click(screen.getByRole("option", { name: /Loja Antiga/ }));

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1/structure");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("escolher outra organização com alterações pendentes pede confirmação antes de sair", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure", { dirty: true });

        await user.click(screen.getByRole("combobox"));
        await user.click(screen.getByRole("option", { name: /Clínica Nova/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("ao abrir, o foco vai para o campo de busca", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));

        expect(screen.getByRole("textbox", { name: /Filtrar organizações/ })).toHaveFocus();
    });

    it("Esc fecha sem navegar e devolve o foco ao gatilho", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        await user.keyboard("{Escape}");

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByRole("combobox")).toHaveFocus();
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("clicar fora fecha o seletor sem devolver o foco", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await user.click(screen.getByTestId("path"));

        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("digitar filtra por nome ou slug, sem diferenciar acento nem caixa", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        await user.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "CLINICA");

        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Loja Antiga/ })).toBeNull();
        expect(screen.queryByRole("option", { name: /Administrar Organizações/ })).toBeNull();

        await user.clear(screen.getByRole("textbox", { name: /Filtrar organizações/ }));
        await user.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "loja-antiga");
        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Clínica Nova/ })).toBeNull();
    });

    it("busca sem resultado mostra a mensagem com o termo", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        await user.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "xyz");

        expect(screen.queryByRole("option")).toBeNull();
        expect(screen.getByRole("status")).toHaveTextContent("Nenhuma organização encontrada para “xyz”");
    });

    it("setas e Enter percorrem a lista e abrem a organização escolhida, devolvendo o foco", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        const campo = screen.getByRole("textbox", { name: /Filtrar organizações/ });
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        const clinica = screen.getByRole("option", { name: /Clínica Nova/ });

        expect(campo).toHaveAttribute("aria-activedescendant", administrar.id);
        await user.keyboard("{ArrowDown}{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        expect(clinica).toHaveAttribute("aria-selected", "true");

        await user.keyboard("{Enter}");

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_2");
        expect(screen.getByRole("combobox")).toHaveFocus();
    });

    it("End e Home levam à última e à primeira opção, e a seta para baixo não passa do fim", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));
        const campo = screen.getByRole("textbox", { name: /Filtrar organizações/ });
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        const clinica = screen.getByRole("option", { name: /Clínica Nova/ });

        await user.keyboard("{End}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        await user.keyboard("{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        await user.keyboard("{Home}");
        expect(campo).toHaveAttribute("aria-activedescendant", administrar.id);
    });

    it("a situação de cada organização fica no nome acessível da opção", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs/org_1/structure");

        await user.click(screen.getByRole("combobox"));

        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toHaveAccessibleName(/Ativa/);
        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toHaveAccessibleName(/Inativa/);
    });

    it("o rodapé mostra a versão da instância e o botão de atalhos abre a lista", async () => {
        const user = userEvent.setup();
        const onOpenShortcuts = vi.fn();
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs", { onOpenShortcuts });

        expect(screen.getByText("POP Harness v0.9.1 · Rust")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Atalhos" })).toBeNull();

        await user.click(screen.getByRole("button", { name: "? atalhos" }));
        expect(onOpenShortcuts).toHaveBeenCalledTimes(1);
    });

    describe("CF-02: rail de 240 px e modo foco de 64 px", () => {
        function buildState(
            compact: boolean,
            onToggleCompact = vi.fn(),
            path = "/orgs",
        ) {
            prepareSession("global", TWO_ORGS);
            return {
                onToggleCompact,
                ...render(
                    <MemoryRouter initialEntries={[path]}>
                        <SessionProvider>
                            <GuardChangesProvider>
                                <Rail
                                    onOpenShortcuts={() => {}}
                                    compact={compact}
                                    onToggleCompact={onToggleCompact}
                                />
                            </GuardChangesProvider>
                        </SessionProvider>
                    </MemoryRouter>,
                ),
            };
        }

        it("expandido: mostra o chip do cluster na lista e o botão de recolher", async () => {
            const user = userEvent.setup();
            const { onToggleCompact } = buildState(false);

            expect(screen.getByText("POP Harness Cluster")).toBeInTheDocument();
            expect(screen.getByText("POP Harness v0.9.1 · Rust")).toBeInTheDocument();
            expect(screen.queryByRole("button", { name: "Expandir menu lateral" })).toBeNull();

            await user.click(screen.getByRole("button", { name: "Recolher menu lateral" }));
            expect(onToggleCompact).toHaveBeenCalledTimes(1);
        });

        it("em foco: o chip do cluster vira um tile de 32×32 com o nome no title", () => {
            buildState(true);

            const chip = screen.getByText("POP Harness Cluster").closest(".poph-rail__cluster");
            expect(chip).toHaveClass("poph-rail__cluster--compact");
            expect(chip).toHaveAttribute("title", "POP Harness Cluster · instância respondendo");
            expect(screen.queryByText("POP Harness v0.9.1 · Rust")).toBeNull();
            expect(screen.getByRole("button", { name: "Expandir menu lateral" })).toBeInTheDocument();
            expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute(
                "title",
                "Organizações",
            );
        });

        it("em foco: com organização aberta, o seletor vira um tile e o popover abre ao lado", async () => {
            const user = userEvent.setup();
            buildState(true, vi.fn(), "/orgs/org_1/structure");

            expect(screen.queryByText("POP Harness Cluster")).toBeNull();
            const selector = screen.getByRole("combobox");
            expect(selector).toHaveAttribute("title", "Loja Antiga");
            expect(selector.closest(".poph-org-select")).toHaveClass("poph-org-select--compact");

            await user.click(selector);
            expect(screen.getByRole("listbox")).toBeInTheDocument();
        });

        it("o botão que alterna continua alcançável por teclado nos dois estados", async () => {
            const user = userEvent.setup();
            const { onToggleCompact } = buildState(true);

            const expandir = screen.getByRole("button", { name: "Expandir menu lateral" });
            expandir.focus();
            expect(expandir).toHaveFocus();
            await user.keyboard("{Enter}");
            expect(onToggleCompact).toHaveBeenCalledTimes(1);
        });
    });
});

describe("CF-07: seletor de agente no topo do rail", () => {
    function agente(partial: Partial<Agente> & Pick<Agente, "id" | "name" | "slug">): Agente {
        return {
            org_id: "org_1",
            description: "",
            status: "active",
            default_department_id: null,
            human_handoff_department_id: null,
            version: 1,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            ...partial,
        };
    }

    const MATRIX = agente({
        id: "agt_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        model: "gemini-3.8-flash",
        status: "active",
    });
    const PATINHAS = agente({
        id: "agt_2",
        name: "Patinhas",
        slug: "patinhas",
        status: "inactive",
    });
    const TRIAGEM = agente({
        id: "agt_3",
        name: "Triagem",
        slug: "triagem",
        model: "gpt-6-sol",
        status: "active",
    });
    const OUTSIDE_LIST = agente({
        id: "agt_9",
        name: "Agente Arquivado",
        slug: "agente-arquivado",
        status: "archived",
    });

    function prepareServer(
        options: {
            agents?: Agente[];
            standalone?: { agente: Agente } | { falha: true };
            failureList?: boolean;
        } = {},
    ) {
        const servidor = new FakeServer();
        const standalone = options.standalone;
        if (standalone && "agente" in standalone) {
            servidor.respond("GET", `/agents/${standalone.agente.id}`, {
                status: 200,
                corpo: standalone.agente,
            });
        } else if (standalone) {
            servidor.respond("GET", "/agents/", () =>
                apiError(404, "not_found", "agente não encontrado"),
            );
        }
        servidor.respond("GET", "/agents", () =>
            options.failureList
                ? apiError(500, "interno", "falha ao ler os agentes")
                : {
                      status: 200,
                      corpo: { items: options.agents ?? [MATRIX, PATINHAS, TRIAGEM], next_cursor: null },
                  },
        );
        servidor.install();
        return servidor;
    }

    function build(path: string, compact = false) {
        prepareSession("global", TWO_ORGS);
        return render(
            <MemoryRouter initialEntries={[path]}>
                <SessionProvider>
                    <GuardChangesProvider>
                        <Rail onOpenShortcuts={() => {}} compact={compact} />
                        <Probe />
                    </GuardChangesProvider>
                </SessionProvider>
            </MemoryRouter>,
        );
    }

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("mostra o link de volta para os agentes da organização", () => {
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Agentes da Org" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
    });

    it("no lugar do seletor de organização, mostra o agente da URL com modelo e situação", async () => {
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("Atendente Matrix");
        expect(gatilho).toHaveTextContent("gemini-3.8-flash · Ativo");
        expect(screen.queryByText("POP Harness Cluster")).toBeNull();
        expect(screen.queryByRole("combobox", { name: /^Organização:/ })).toBeNull();
    });

    it("agente sem modelo próprio mostra o padrão do servidor e a situação", async () => {
        prepareServer();
        build("/orgs/org_1/agents/agt_2/files");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Patinhas" });
        expect(gatilho).toHaveTextContent("modelo padrão do servidor · Inativo");
    });

    it("abre a lista dos agentes da organização com busca por nome", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));

        const campo = screen.getByRole("textbox", { name: /Filtrar agentes/ });
        expect(campo).toHaveFocus();
        expect(screen.getAllByRole("option")).toHaveLength(3);

        await user.type(campo, "TRIAGE");
        expect(screen.getByRole("option", { name: /Triagem/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Patinhas/ })).toBeNull();

        await user.clear(campo);
        await user.type(campo, "patinhas");
        expect(screen.getByRole("option", { name: /Patinhas/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Triagem/ })).toBeNull();
    });

    it("marca o agente aberto com o check e não navega ao escolhê-lo", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        const current = screen.getByRole("option", { name: /Atendente Matrix/ });
        expect(current).toHaveAttribute("aria-current", "true");

        await user.click(current);

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByTestId("path")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("trocar de agente com alterações pendentes pede confirmação antes de sair", async () => {
        const user = userEvent.setup();
        prepareServer();
        prepareSession("global", TWO_ORGS);
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <SessionProvider>
                    <GuardChangesProvider>
                        <Rail onOpenShortcuts={() => {}} />
                        <Dirty />
                        <Probe />
                    </GuardChangesProvider>
                </SessionProvider>
            </MemoryRouter>,
        );

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        await user.click(screen.getByRole("option", { name: /Triagem/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("setas e Enter trocam de agente, indo para as Instruções dele", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        const campo = screen.getByRole("textbox", { name: /Filtrar agentes/ });
        const triagem = screen.getByRole("option", { name: /Triagem/ });

        expect(campo).toHaveAttribute("aria-activedescendant", screen.getByRole("option", { name: /Atendente Matrix/ }).id);
        await user.keyboard("{ArrowDown}{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", triagem.id);

        await user.keyboard("{Enter}");

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_1/agents/agt_3/instructions");
        expect(screen.getByRole("combobox")).toHaveFocus();
    });

    it("Esc fecha sem navegar e devolve o foco ao gatilho", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        await user.keyboard("{Escape}");

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByRole("combobox")).toHaveFocus();
        expect(screen.getByTestId("path")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("clicar fora fecha o seletor", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await user.click(screen.getByTestId("path"));

        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("organização sem agentes mostra o estado vazio no popover", async () => {
        const user = userEvent.setup();
        prepareServer({ agents: [] });
        build("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox"));
        expect(screen.getByRole("status")).toHaveTextContent("Nenhum agente nesta organização");
    });

    it("falha ao ler os agentes aparece no popover e não derruba a navegação", async () => {
        const user = userEvent.setup();
        prepareServer({ failureList: true });
        build("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox");
        expect(gatilho).toHaveAccessibleName("Agente: Agentes indisponíveis");
        await user.click(gatilho);
        expect(await screen.findByRole("status")).toHaveTextContent(
            "Não foi possível ler os agentes",
        );
        expect(screen.getByRole("link", { name: "Chat de Teste" })).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes da Org" })).toBeInTheDocument();
    });

    it("agente da URL fora da lista é lido sozinho e mostra o nome dele", async () => {
        prepareServer({ standalone: { agente: OUTSIDE_LIST } });
        build("/orgs/org_1/agents/agt_9/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Agente Arquivado" });
        expect(gatilho).toHaveTextContent("Agente Arquivado");
        expect(gatilho).toHaveTextContent("Arquivado");
    });

    it("agente da URL que não existe mais aparece como não encontrado, sem quebrar a tela", async () => {
        prepareServer({ standalone: { falha: true } });
        build("/orgs/org_1/agents/agt_9/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Agente não encontrado" });
        expect(gatilho).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes da Org" })).toBeInTheDocument();
    });

    it("uma consulta por organização: abrir a lista duas vezes não relê os agentes", async () => {
        const user = userEvent.setup();
        const servidor = prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        await user.click(gatilho);
        await user.keyboard("{Escape}");
        await user.click(gatilho);

        expect(servidor.callsFor("GET", "/agents")).toHaveLength(1);
    });

    it("em modo foco, o link e o gatilho viram tiles e o popover abre ao lado", async () => {
        const user = userEvent.setup();
        prepareServer();
        build("/orgs/org_1/agents/agt_1/instructions", true);

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        expect(gatilho).toHaveAttribute("title", "Atendente Matrix");
        expect(gatilho.closest(".poph-agent-select")).toHaveClass("poph-agent-select--compact");

        const goBack = screen.getByRole("link", { name: "Agentes da Org" });
        expect(goBack).toHaveClass("poph-agent-select__return--compact");

        await user.click(gatilho);
        expect(screen.getByRole("listbox")).toBeInTheDocument();
    });

    it("com token de organização, o seletor do agente continua funcionando", async () => {
        prepareSession("org", [{ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" }]);
        prepareServer();
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <SessionProvider>
                    <GuardChangesProvider>
                        <Rail onOpenShortcuts={() => {}} />
                        <Probe />
                    </GuardChangesProvider>
                </SessionProvider>
            </MemoryRouter>,
        );

        expect(
            await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }),
        ).toBeEnabled();
    });

    it("fora das rotas de agente, o topo continua sendo o seletor de organização", () => {
        prepareServer();
        build("/orgs/org_1/structure");

        expect(screen.getByRole("combobox", { name: "Organização: Loja Antiga" })).toBeInTheDocument();
        expect(screen.queryByRole("link", { name: "Agentes da Org" })).toBeNull();
    });
});

describe("CF-09: cockpit do agente no rail", () => {
    const AGENTE: Agente = {
        id: "agt_1",
        org_id: "org_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        description: "",
        status: "active",
        model: "gemini-3.8-flash",
        default_department_id: null,
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    };

    function prepareServer() {
        const servidor = new FakeServer();
        servidor.respond("GET", "/agents", {
            status: 200,
            corpo: { items: [AGENTE], next_cursor: null },
        });
        servidor.install();
        return servidor;
    }

    function build(path: string, options: { compact?: boolean; dirty?: boolean } = {}) {
        prepareSession("global", TWO_ORGS);
        return render(
            <MemoryRouter initialEntries={[path]}>
                <SessionProvider>
                    <GuardChangesProvider>
                        <Rail onOpenShortcuts={() => {}} compact={options.compact ?? false} />
                        {options.dirty && <Dirty />}
                        <Probe />
                    </GuardChangesProvider>
                </SessionProvider>
            </MemoryRouter>,
        );
    }

    const ROUTE = "/orgs/org_1/agents/agt_1/instructions";

    it("nas rotas de agente, o rail tem os itens da prancha 03 na ordem do desenho", () => {
        prepareServer();
        build(ROUTE);

        const labels = [
            "Instruções & Prompts",
            "Base de Conhecimento",
            "Arquivos do Agente",
            "Roteamento & Habilidades",
            "Modelo & Parâmetros (Jev)",
            "Chat de Teste",
            "Inspeção de Turnos",
        ];
        const list = screen.getByRole("navigation").querySelector(".sofa-sidebar-rail__list");
        const ordem = Array.from(list?.querySelectorAll(".sofa-rail-item__label") ?? []).map(
            (no) => no.textContent,
        );
        expect(ordem).toEqual(labels);
        expect(screen.getByText("Diagnóstico & Teste")).toBeInTheDocument();
        expect(screen.queryByText("TENANTS E RECURSOS")).toBeNull();
        expect(screen.queryByText("SISTEMA")).toBeNull();
    });

    it("cada item ativo leva para a tela real do agente", () => {
        prepareServer();
        build(ROUTE);

        const routes: [string, string][] = [
            ["Instruções & Prompts", "instructions"],
            ["Base de Conhecimento", "knowledge-base"],
            ["Arquivos do Agente", "files"],
            ["Roteamento & Habilidades", "routing"],
            ["Modelo & Parâmetros (Jev)", "model-jev"],
            ["Chat de Teste", "test-chat"],
        ];
        for (const [label, tab] of routes) {
            expect(screen.getByRole("link", { name: label })).toHaveAttribute(
                "href",
                `/orgs/org_1/agents/agt_1/${tab}`,
            );
        }
    });

    it("o item da rota atual fica destacado, e só ele", () => {
        prepareServer();
        build("/orgs/org_1/agents/agt_1/routing");

        expect(screen.getByRole("link", { name: "Roteamento & Habilidades" })).toHaveAttribute(
            "aria-current",
            "page",
        );
        expect(screen.getByRole("link", { name: "Instruções & Prompts" })).not.toHaveAttribute(
            "aria-current",
        );
    });

    it("na raiz do agente, Instruções & Prompts já aparece ativa", () => {
        prepareServer();
        build("/orgs/org_1/agents/agt_1");

        const link = screen.getByRole("link", { name: "Instruções & Prompts" });
        expect(link.querySelector(".sofa-rail-item")).toHaveAttribute("aria-current", "true");
    });

    it("toda rota do cockpit existe no App.tsx", () => {
        const declared = Array.from(AppSource.matchAll(/path="([^"]+)"/g), (finding) => finding[1] ?? "");
        for (const entry of ITEMS_COCKPIT_AGENT) {
            const route = entry.route({ orgId: "org_1", agentId: "agt_1", totalOrgs: null });
            if (route === null) continue;
            const fallback = route
                .replace("/orgs/org_1", "/orgs/:orgId")
                .replace("/agents/agt_1", "/agents/:agentId");
            expect(declared).toContain(fallback);
        }
    });

    it("Inspeção de Turnos continua 'em breve', desabilitada e sem navegar", async () => {
        const user = userEvent.setup();
        prepareServer();
        build(ROUTE);

        expect(screen.queryByRole("link", { name: "Inspeção de Turnos" })).toBeNull();
        const button = screen.getByRole("button", { name: /Inspeção de Turnos/ });
        expect(button).toBeDisabled();
        expect(button).toHaveTextContent("em breve");

        await user.click(button);
        expect(screen.getByTestId("path")).toHaveTextContent(ROUTE);
    });

    it("o rodapé mostra 'Agente: <nome>' no lugar da versão", async () => {
        prepareServer();
        build(ROUTE);

        expect(await screen.findByText("Agente: Atendente Matrix")).toBeInTheDocument();
        expect(screen.queryByText("POP Harness v0.9.1 · Rust")).toBeNull();
    });

    it("no modo foco os itens viram tiles com title e o título de seção sai de cena", () => {
        prepareServer();
        build(ROUTE, { compact: true });

        expect(screen.queryByText("Diagnóstico & Teste")).toBeNull();
        expect(screen.queryByText(/^Agente: /)).toBeNull();
        expect(screen.getByRole("link", { name: "Instruções & Prompts" })).toHaveAttribute(
            "title",
            "Instruções & Prompts",
        );
        const emBreve = screen.getByRole("button", { name: /Inspeção de Turnos/ });
        expect(emBreve.closest(".poph-rail__blocked")).toHaveAttribute(
            "title",
            "Inspeção de Turnos",
        );
    });

    it("trocar de tela pelo cockpit com alterações pendentes pede confirmação", async () => {
        const user = userEvent.setup();
        prepareServer();
        build(ROUTE, { dirty: true });

        await user.click(screen.getByRole("link", { name: "Arquivos do Agente" }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent(ROUTE);
    });

    it("fora das rotas de agente, o rail continua com os grupos e itens de sempre", () => {
        prepareServer();
        build("/orgs/org_1/structure");

        expect(screen.getByText("TENANTS E RECURSOS")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes" })).toBeInTheDocument();
        expect(screen.queryByText("Instruções & Prompts")).toBeNull();
        expect(screen.queryByText("Diagnóstico & Teste")).toBeNull();
    });
});

describe("I18N-04: rail e seletores em en-US", () => {
    const AGENT_EN: Agente = {
        id: "agt_1",
        org_id: "org_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        description: "",
        status: "active",
        model: "gemini-3.8-flash",
        default_department_id: null,
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    };

    function serveAgents(agents: Agente[] = [AGENT_EN]) {
        return new FakeServer()
            .respond("GET", "/agents", {
                status: 200,
                corpo: { items: agents, next_cursor: null },
            })
            .install();
    }

    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista de navegação tem os mesmos itens em inglês", () => {
        setLocale("en-US");

        expect(ITEMS_NAVIGATION.map((entry) => entry.caption)).toEqual([
            "Organizations",
            "Structure",
            "Products and catalog",
            "Agents",
            "Model and Jev",
            "Turn inspection",
            "Test chat",
            "WhatsApp and channels",
            "Matrix (Element)",
            "Plugins and extensions",
            "Instance health",
        ]);
        expect(ITEMS_COCKPIT_AGENT.map((entry) => entry.caption)).toContain(
            "Instructions & Prompts",
        );
    });

    it("os grupos e o item 'em breve' aparecem em inglês no rail", () => {
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs");

        expect(screen.getByText("TENANTS AND RESOURCES")).toBeInTheDocument();
        expect(screen.getByText("RUNTIME AND ENGINE")).toBeInTheDocument();
        expect(screen.getByText("SYSTEM")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: /^Organizations/ })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Turn inspection/ })).toHaveTextContent(
            "coming soon",
        );
    });

    it("o rail reage à troca de idioma sem recarregar", () => {
        prepareSession("global", TWO_ORGS);
        buildRail("/orgs");

        expect(screen.getByRole("link", { name: /^Organizações/ })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("link", { name: /^Organizations/ })).toBeInTheDocument();
        expect(screen.queryByRole("link", { name: /^Organizações/ })).toBeNull();
    });

    it("o seletor de organização fala inglês no gatilho, na busca e nas opções", async () => {
        const user = userEvent.setup();
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/structure");

        const gatilho = screen.getByRole("combobox", {
            name: "Organization: Loja Antiga",
        });
        await user.click(gatilho);

        expect(screen.getByPlaceholderText("Filter organizations…")).toHaveAccessibleName(
            "Filter organizations by name or slug",
        );
        expect(screen.getByRole("listbox", { name: "Available organizations" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: /Manage Organizations/ })).toBeInTheDocument();
    });

    it("o seletor de agente e o link de volta falam inglês", async () => {
        serveAgents();
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Org Agents" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
        const gatilho = await screen.findByRole("combobox", { name: "Agent: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("gemini-3.8-flash · Active");
    });

    it("agente sem modelo próprio mostra o padrão do servidor em inglês", async () => {
        serveAgents([{ ...AGENT_EN, model: undefined, status: "inactive" }]);
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/agents/agt_1/files");

        const gatilho = await screen.findByRole("combobox", { name: "Agent: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("server default model · Inactive");
    });

    it("organização sem agentes mostra o estado vazio em inglês", async () => {
        const user = userEvent.setup();
        serveAgents([]);
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/agents/agt_1/instructions");

        await user.click(await screen.findByRole("combobox"));
        expect(screen.getByRole("status")).toHaveTextContent("No agent in this organization");
    });

    it("o rodapé do cockpit, os atalhos e o botão de recolher falam inglês", async () => {
        serveAgents();
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/agents/agt_1/instructions");

        expect(await screen.findByText("Agent: Atendente Matrix")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "? shortcuts" })).toHaveAttribute(
            "title",
            "Keyboard shortcuts (?)",
        );
        expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeInTheDocument();
    });

    it("o cockpit do agente tem as abas e o grupo de diagnóstico em inglês", () => {
        serveAgents();
        prepareSession("global", TWO_ORGS);
        setLocale("en-US");
        buildRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Knowledge Base" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/knowledge-base",
        );
        expect(screen.getByText("Diagnostics & Testing")).toBeInTheDocument();
    });
});
