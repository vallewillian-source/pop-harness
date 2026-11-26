
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { I18nProvider, setLocale } from "../i18n";

afterEach(() => {
    setLocale("pt-BR");
});
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { AgentIntegrations } from "./AgentIntegrations";

const ORG = "org_1";
const AGENTE = "ag_1";
const LIST = `/agents/${AGENTE}/integrations`;

const AGENT_JSON = {
    id: AGENTE,
    org_id: ORG,
    name: "Beatriz",
    slug: "beatriz",
    description: "",
    status: "active",
    default_department_id: "dep_1",
    human_handoff_enabled: false,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

type Integracao = {
    id: string;
    kind: string;
    status: string;
    connection_state: string;
    version: number;
    tem_segredo: boolean;
    configuracao?: Record<string, unknown> | null;
    segredos_preenchidos?: string[] | null;
};

function integracao(partial: Partial<Integracao> & { id: string }): Integracao {
    return {
        kind: "whatsapp",
        status: "active",
        connection_state: "connected",
        version: 1,
        tem_segredo: false,
        configuracao: null,
        segredos_preenchidos: [],
        ...partial,
    };
}

const MANIFEST_ECHO = {
    nome: "Eco (exemplo)",
    kind: "eco",
    versao: "0.1.0",
    protocolo: 1,
    configuracao: {
        type: "object",
        properties: {
            modo: {
                type: "string",
                enum: ["inverter"],
                default: "inverter",
                description: "O eco só tem o modo que inverte o texto.",
            },
            remetente: {
                type: "string",
                minLength: 1,
                title: "Nome do remetente",
                description: "Nome que o eco assina nas respostas.",
            },
        },
        required: ["remetente"],
    },
    segredos: {
        type: "object",
        properties: {
            token: {
                type: "string",
                title: "Token do canal",
                description: "Credencial que o eco receberia do canal no handshake.",
            },
        },
        required: ["token"],
    },
};

const MANIFEST_SMS = {
    nome: "SMS (exemplo)",
    kind: "sms",
    versao: "1.0.0",
    protocolo: 1,
    configuracao: {
        type: "object",
        properties: {
            remetente: { type: "string", title: "Remetente" },
            tentativas: { type: "integer", minimum: 1, maximum: 5, default: 3 },
        },
        required: ["remetente"],
    },
    segredos: {
        type: "object",
        properties: { chave: { type: "string", title: "Chave" } },
    },
};

const MANIFESTS: Record<string, unknown> = { eco: MANIFEST_ECHO, sms: MANIFEST_SMS };

function build() {
    return render(
        <I18nProvider>
            <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/integrations`]}>
                <GuardChangesProvider>
                    <Routes>
                        <Route
                            path="/orgs/:orgId/agents/:agentId/integrations"
                            element={<AgentIntegrations />}
                        />
                        <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                    </Routes>
                </GuardChangesProvider>
            </MemoryRouter>
        </I18nProvider>,
    );
}

async function chooseInMenu(
    user: ReturnType<typeof userEvent.setup>,
    gatilho: string,
    acao: string,
) {
    await user.click(await screen.findByRole("button", { name: gatilho }));
    await user.click(await screen.findByRole("menuitem", { name: acao }));
}

function serverWith(
    initial: Integracao[],
    options: {
        onCreate?: () => { status: number; corpo?: unknown };
        onChange?: () => { status: number; corpo?: unknown };
        onQrcode?: () => { status: number; corpo?: unknown };
        onState?: () => { status: number; corpo?: unknown };
        onDisconnect?: (call: { headers: Record<string, string> }) => {
            status: number;
            corpo?: unknown;
        };
        onManifest?: (kind: string) => { status: number; corpo?: unknown } | undefined;
        connected?: boolean;
    } = {},
) {
    const estado = { itens: [...initial], opened: options.connected ?? false, pairings: 0 };
    const pathId = (url: string) => url.split(`${LIST}/`)[1]?.split("/")[0] ?? "";
    const servidor = new FakeServer()
        .respond("GET", "/manifests/", (call) => {
            const kind = call.url.split("/manifests/")[1] ?? "";
            const own = options.onManifest?.(kind);
            if (own) return own;
            const manifesto = MANIFESTS[kind];
            return manifesto
                ? { status: 200, corpo: manifesto }
                : apiError(404, "not_found", `manifesto \`${kind}\` não encontrado`);
        })
        .respond("POST", "/whatsapp/qrcode", (call) => {
            if (options.onQrcode) return options.onQrcode();
            const id = pathId(call.url);
            const current = estado.itens.find((item) => item.id === id);
            if (!current) return apiError(404, "not_found", "integração não encontrada");
            if (estado.opened) {
                estado.itens = estado.itens.map((item) =>
                    item.id === id ? { ...item, connection_state: "connected" } : item,
                );
                return {
                    status: 200,
                    corpo: {
                        integration_id: id,
                        instance: "loja",
                        state: "open",
                        connection_state: "connected",
                        qrcode: null,
                        pairing_code: null,
                    },
                };
            }
            estado.pairings += 1;
            estado.itens = estado.itens.map((item) =>
                item.id === id ? { ...item, connection_state: "awaiting_scan" } : item,
            );
            return {
                status: 200,
                corpo: {
                    integration_id: id,
                    instance: "loja",
                    state: "connecting",
                    connection_state: "awaiting_scan",
                    qrcode: {
                        base64: `data:image/png;base64,QR-FALSO-${estado.pairings}`,
                        code: "2@codigo-falso",
                        count: estado.pairings,
                    },
                    expira_em: null,
                    pairing_code: "ABCD-1234",
                },
            };
        })
        .respond("GET", "/whatsapp/state", (call) => {
            if (options.onState) return options.onState();
            const id = pathId(call.url);
            const connection_state = estado.opened ? "connected" : "disconnected";
            estado.itens = estado.itens.map((item) =>
                item.id === id ? { ...item, connection_state } : item,
            );
            return {
                status: 200,
                corpo: {
                    integration_id: id,
                    instance: "loja",
                    state: estado.opened ? "open" : "close",
                    connection_state,
                },
            };
        })
        .respond("POST", "/whatsapp/disconnect", (call) => {
            if (options.onDisconnect) return options.onDisconnect(call);
            const id = pathId(call.url);
            estado.opened = false;
            estado.itens = estado.itens.map((item) =>
                item.id === id ? { ...item, connection_state: "disconnected" } : item,
            );
            return {
                status: 200,
                corpo: {
                    integration_id: id,
                    instance: "loja",
                    connection_state: "disconnected",
                    disconnected: true,
                },
            };
        })
        .respond("PATCH", `${LIST}/`, (call) => {
            if (options.onChange) return options.onChange();
            const id = call.url.split(`${LIST}/`)[1] ?? "";
            const corpo = call.corpo as {
                status?: string;
                config_json?: Record<string, unknown>;
                secrets?: Record<string, string>;
            };
            const current = estado.itens.find((item) => item.id === id);
            if (!current) return apiError(404, "not_found", "integração não encontrada");
            const filled = corpo.secrets
                ? [
                      ...new Set([
                          ...(current.segredos_preenchidos ?? []),
                          ...Object.keys(corpo.secrets),
                      ]),
                  ].filter((nome) => (corpo.secrets![nome] ?? "x") !== "")
                : (current.segredos_preenchidos ?? []);
            const changed = {
                ...current,
                status: corpo.status ?? current.status,
                configuracao: corpo.config_json ?? current.configuracao ?? null,
                segredos_preenchidos: filled,
                tem_segredo: filled.length > 0 || current.tem_segredo,
                version: current.version + 1,
            };
            estado.itens = estado.itens.map((item) => (item.id === id ? changed : item));
            return { status: 200, corpo: changed };
        })
        .respond("GET", `${LIST}/`, (call) => {
            const id = call.url.split(`${LIST}/`)[1] ?? "";
            const found = estado.itens.find((item) => item.id === id);
            return found
                ? { status: 200, corpo: found }
                : apiError(404, "not_found", "integração não encontrada");
        })
        .respond("DELETE", `${LIST}/`, (call) => {
            const id = call.url.split(`${LIST}/`)[1] ?? "";
            estado.itens = estado.itens.filter((item) => item.id !== id);
            return { status: 200, corpo: { deleted: true } };
        })
        .respond("POST", LIST, (call) => {
            if (options.onCreate) return options.onCreate();
            const corpo = call.corpo as {
                kind: string;
                status?: string;
                config_json?: Record<string, unknown>;
                secrets?: Record<string, string>;
            };
            const created = integracao({
                id: "int_nova",
                kind: corpo.kind,
                status: corpo.status ?? "active",
                connection_state: "disconnected",
                configuracao: corpo.config_json ?? null,
                segredos_preenchidos: corpo.secrets ? Object.keys(corpo.secrets) : [],
                tem_segredo: !!corpo.secrets,
            });
            estado.itens = [...estado.itens, created];
            return { status: 201, corpo: created };
        })
        .respond("GET", LIST, () => ({
            status: 200,
            corpo: { items: estado.itens, next_cursor: null },
        }))
        .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENT_JSON })
        .respond("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return { servidor: servidor.install(), estado };
}

describe("Aba Integrações", () => {
    it("lista tipo, situação, conexão e indica o segredo sem exibir o valor", async () => {
        serverWith([
            integracao({ id: "int_1", tem_segredo: true, connection_state: "connected" }),
            integracao({
                id: "int_2",
                kind: "eco",
                status: "inactive",
                connection_state: "awaiting_scan",
            }),
        ]);
        build();

        expect(await screen.findByText("whatsapp")).toBeInTheDocument();
        expect(screen.getByText("eco")).toBeInTheDocument();
        expect(screen.getAllByText("Ativa").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Inativa").length).toBeGreaterThan(0);
        expect(screen.getByText("gravado (não exibido)")).toBeInTheDocument();
        expect(screen.getByText("sem segredo")).toBeInTheDocument();
        expect(screen.getAllByText("Conectado").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Aguardando leitura do QR").length).toBeGreaterThan(0);
        expect(screen.getByText("Configuração pelo formulário do plugin")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desconectar aparelho" })).toBeInTheDocument();
        expect(screen.getByText("WhatsApp: pareamento e conexão")).toBeInTheDocument();

        const table = screen.getByRole("table", { name: "Integrações do agente" });
        expect(within(table).getByRole("columnheader", { name: "Conexão" })).toBeInTheDocument();
        expect(
            within(table).getByRole("button", { name: "Ações de whatsapp (int_1)" }),
        ).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).not.toBeInTheDocument();
    });

    it("cria enviando só tipo e situação e mostra a integração na lista", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([]);
        build();

        expect(await screen.findByText("Nenhuma integração neste agente")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");
        await user.selectOptions(within(modal).getByLabelText(/Situação/), "inactive");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.callsFor("POST", LIST)).toHaveLength(1));
        const send = servidor.callsFor("POST", LIST)[0]!;
        expect(send.corpo).toEqual({ kind: "whatsapp", status: "inactive" });
        expect(await screen.findByText("Integração whatsapp criada.")).toBeInTheDocument();
        expect(await screen.findByText("whatsapp")).toBeInTheDocument();
    });

    it("não chama a API quando o tipo está vazio", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([]);
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(
            await within(modal).findByText("informe o tipo da integração (por exemplo, whatsapp)"),
        ).toBeInTheDocument();
        expect(servidor.callsFor("POST", LIST)).toHaveLength(0);
    });

    it("mostra o motivo real da API quando a criação é recusada", async () => {
        const user = userEvent.setup();
        serverWith([], {
            onCreate: () =>
                apiError(422, "manifest_indisponivel", "configuração exige o manifesto", {
                    field: "kind",
                    reason: "tipo recusado pelo servidor",
                }),
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(await within(modal).findByText("tipo recusado pelo servidor")).toBeInTheDocument();
    });

    it("no 412 relê a versão, mantém a escolha e salva de novo na versão atual", async () => {
        const user = userEvent.setup();
        let first = true;
        const { servidor, estado } = serverWith([integracao({ id: "int_1", version: 1 })], {
            onChange: () => {
                if (first) {
                    first = false;
                    estado.itens = estado.itens.map((item) =>
                        item.id === "int_1" ? { ...item, version: 5 } : item,
                    );
                    return apiError(412, "version_conflict", "conflito de versão", {
                        current_version: 5,
                        expected_version: 1,
                    });
                }
                const current = estado.itens.find((item) => item.id === "int_1")!;
                const changed = { ...current, status: "inactive", version: current.version + 1 };
                estado.itens = estado.itens.map((item) =>
                    item.id === "int_1" ? changed : item,
                );
                return { status: 200, corpo: changed };
            },
        });
        build();

        await chooseInMenu(user, "Ações de whatsapp (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.selectOptions(within(modal).getByLabelText(/Situação/), "inactive");
        await user.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        expect(await within(modal).findByText(/Versão desatualizada/)).toBeInTheDocument();
        expect(within(modal).getByText(/agora na versão 5/)).toBeInTheDocument();
        expect(within(modal).getByLabelText(/Situação/)).toHaveValue("inactive");

        await user.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        await waitFor(() =>
            expect(servidor.callsFor("PATCH", `${LIST}/int_1`)).toHaveLength(2),
        );
        const second = servidor.callsFor("PATCH", `${LIST}/int_1`)[1]!;
        expect(second.headers["if-match"]).toBe("5");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("exclui com confirmação pelo tipo e sem force", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([integracao({ id: "int_1" })]);
        build();

        await chooseInMenu(user, "Ações de whatsapp (int_1)", "Excluir");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        const confirmAction = within(modal).getByRole("button", { name: "Excluir integração" });
        expect(confirmAction).toBeDisabled();

        await user.type(within(modal).getByLabelText(/Digite whatsapp/), "whatsapp");
        expect(confirmAction).toBeEnabled();
        await user.click(confirmAction);

        await waitFor(() =>
            expect(servidor.callsFor("DELETE", `${LIST}/int_1`)).toHaveLength(1),
        );
        const deletion = servidor.callsFor("DELETE", `${LIST}/int_1`)[0]!;
        expect(deletion.url).not.toContain("force");
        expect(
            await screen.findByText("Nenhuma integração neste agente"),
        ).toBeInTheDocument();
    });
});

describe("Formulário do manifesto na integração (HR-44e)", () => {
    it("desenha o formulário do tipo a partir do manifesto, com obrigatório e segredo mascarado", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "eco");

        const remetente = await within(modal).findByLabelText(/Nome do remetente/);
        expect(remetente).toHaveValue("");
        expect(within(modal).getByText(/Configuração do plugin “Eco \(exemplo\)”/)).toBeInTheDocument();
        const token = within(modal).getByLabelText(/Token do canal/);
        expect(token).toHaveAttribute("type", "password");
        expect(token).toHaveValue("");
        expect(within(modal).getByLabelText(/Modo/)).toHaveValue("inverter");
        expect(remetente).toBeRequired();
    });

    it("tipo sem manifesto avisa e cria só com tipo e situação", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([]);
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");

        expect(await within(modal).findByText("Este tipo não tem formulário")).toBeInTheDocument();
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.callsFor("POST", LIST)).toHaveLength(1));
        expect(servidor.callsFor("POST", LIST)[0]!.corpo).toEqual({
            kind: "whatsapp",
            status: "active",
        });
    });

    it("envia config_json com os tipos do esquema e não manda segredo em branco", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([]);
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "sms");
        await user.type(await within(modal).findByLabelText(/Remetente/), "5511999999999");
        await user.clear(within(modal).getByLabelText(/Tentativas/));
        await user.type(within(modal).getByLabelText(/Tentativas/), "4");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.callsFor("POST", LIST)).toHaveLength(1));
        const send = servidor.callsFor("POST", LIST)[0]!;
        expect(send.corpo).toEqual({
            kind: "sms",
            status: "active",
            config_json: { remetente: "5511999999999", tentativas: 4 },
        });
    });

    it("manda o segredo digitado e o obrigatório é cobrado antes de chamar a API", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([]);
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "eco");
        await user.type(await within(modal).findByLabelText(/Nome do remetente/), "Loja");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(await within(modal).findByText("informe o segredo")).toBeInTheDocument();
        expect(servidor.callsFor("POST", LIST)).toHaveLength(0);

        await user.type(within(modal).getByLabelText(/Token do canal/), "segredo-do-teste");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.callsFor("POST", LIST)).toHaveLength(1));
        const send = servidor.callsFor("POST", LIST)[0]!;
        expect(send.corpo).toEqual({
            kind: "eco",
            status: "active",
            config_json: { modo: "inverter", remetente: "Loja" },
            secrets: { token: "segredo-do-teste" },
        });
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        expect(screen.queryByText("segredo-do-teste")).not.toBeInTheDocument();
    });

    it("põe o erro de validação da API no campo que ela aponta", async () => {
        const user = userEvent.setup();
        serverWith([], {
            onCreate: () =>
                apiError(400, "validation_error", "configuração inválida", {
                    field: "config_json.remetente",
                    reason: "precisa de pelo menos 3 caractere(s)",
                }),
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "eco");
        await user.type(await within(modal).findByLabelText(/Nome do remetente/), "Loja");
        await user.type(within(modal).getByLabelText(/Token do canal/), "chave");
        await user.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(
            await within(modal).findByText("precisa de pelo menos 3 caractere(s)"),
        ).toBeInTheDocument();
        expect(within(modal).getByLabelText(/Nome do remetente/)).toHaveValue("Loja");
    });

    it("no 412 relê a versão e mantém configuração e segredo digitados nesta sessão", async () => {
        const user = userEvent.setup();
        let first = true;
        const { servidor, estado } = serverWith(
            [
                integracao({
                    id: "int_1",
                    kind: "eco",
                    version: 1,
                    tem_segredo: true,
                    configuracao: { modo: "inverter", remetente: "Antigo" },
                    segredos_preenchidos: ["token"],
                }),
            ],
            {
                onChange: () => {
                    if (first) {
                        first = false;
                        estado.itens = estado.itens.map((item) =>
                            item.id === "int_1" ? { ...item, version: 5 } : item,
                        );
                        return apiError(412, "version_conflict", "conflito de versão", {
                            current_version: 5,
                            expected_version: 1,
                        });
                    }
                    const current = estado.itens.find((item) => item.id === "int_1")!;
                    const changed = { ...current, version: current.version + 1 };
                    estado.itens = estado.itens.map((item) =>
                        item.id === "int_1" ? changed : item,
                    );
                    return { status: 200, corpo: changed };
                },
            },
        );
        build();

        await chooseInMenu(user, "Ações de eco (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        const remetente = await within(modal).findByLabelText(/Nome do remetente/);
        expect(remetente).toHaveValue("Antigo");
        await user.clear(remetente);
        await user.type(remetente, "Novo remetente");
        await user.type(within(modal).getByLabelText(/Token do canal/), "chave-nova");
        await user.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        expect(await within(modal).findByText(/Versão desatualizada/)).toBeInTheDocument();
        expect(within(modal).getByText(/agora na versão 5/)).toBeInTheDocument();
        expect(within(modal).getByLabelText(/Nome do remetente/)).toHaveValue("Novo remetente");
        expect(within(modal).getByLabelText(/Token do canal/)).toHaveValue("chave-nova");

        await user.click(within(modal).getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() =>
            expect(servidor.callsFor("PATCH", `${LIST}/int_1`)).toHaveLength(2),
        );
        const second = servidor.callsFor("PATCH", `${LIST}/int_1`)[1]!;
        expect(second.headers["if-match"]).toBe("5");
        expect(second.corpo).toEqual({
            status: "active",
            config_json: { modo: "inverter", remetente: "Novo remetente" },
            secrets: { token: "chave-nova" },
        });
    });

    it("segredos_preenchidos null diz que há segredo ilegível e cobra regravar", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([
            integracao({
                id: "int_1",
                kind: "eco",
                tem_segredo: true,
                configuracao: { modo: "inverter", remetente: "Loja" },
                segredos_preenchidos: null,
            }),
        ]);
        build();

        await chooseInMenu(user, "Ações de eco (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();

        expect(
            await within(modal).findByText(/a chave de criptografia atual do servidor não consegue lê-lo/),
        ).toBeInTheDocument();
        expect(within(modal).getByText(/A tela não sabe qual segredo está gravado/)).toBeInTheDocument();

        await user.click(within(modal).getByRole("button", { name: "Salvar alterações" }));
        expect(await within(modal).findByText("informe o segredo")).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", `${LIST}/int_1`)).toHaveLength(0);
    });

    it("plugin que não respondeu ao handshake explica e não deixa salvar", async () => {
        const user = userEvent.setup();
        serverWith([], {
            onManifest: () => ({
                status: 503,
                corpo: {
                    code: "manifesto_indisponivel",
                    message: "o plugin deste tipo não respondeu ao handshake",
                    details: { kind: "eco" },
                },
            }),
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.type(within(modal).getByLabelText(/Tipo da integração/), "eco");

        expect(
            await within(modal).findByText(/não respondeu ao handshake/),
        ).toBeInTheDocument();
        expect(within(modal).getByText(/código: manifesto_indisponivel/)).toBeInTheDocument();
        expect(within(modal).getByRole("button", { name: "Criar integração" })).toBeDisabled();
    });
});

describe("Painel do WhatsApp (HR-44d)", () => {
    it("gera o QR com If-Match, desenha o base64 e mostra a rodada e o código de pareamento", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        build();

        await user.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        const image = await screen.findByRole("img", { name: "QR de pareamento do WhatsApp" });
        expect(image).toHaveAttribute("src", "data:image/png;base64,QR-FALSO-1");
        expect(screen.getByText("ABCD-1234")).toBeInTheDocument();
        expect(screen.getByText("rodada 1 do QR")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Gerar outro QR" })).toBeInTheDocument();

        const calls = servidor.callsFor("POST", "/whatsapp/qrcode");
        expect(calls).toHaveLength(1);
        expect(calls[0]!.headers["if-match"]).toBe("1");
        expect(calls[0]!.raw).toBeUndefined();
    });

    it("pareia por número quando o operador digita e recusa número fora do formato", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        build();

        const campo = await screen.findByLabelText(/Número do WhatsApp/);
        await user.type(campo, "119");
        await user.click(screen.getByRole("button", { name: "Conectar / gerar QR" }));
        expect(
            await screen.findByText("use só dígitos, com DDI e DDD (ex.: 5511999998888)"),
        ).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/whatsapp/qrcode")).toHaveLength(0);

        await user.clear(campo);
        await user.type(campo, "5511999998888");
        await user.click(screen.getByRole("button", { name: "Conectar / gerar QR" }));
        await waitFor(() =>
            expect(servidor.callsFor("POST", "/whatsapp/qrcode")).toHaveLength(1),
        );
        expect(servidor.callsFor("POST", "/whatsapp/qrcode")[0]!.corpo).toEqual({
            number: "5511999998888",
        });
    });

    it("mostra “aparelho conectado” sem QR quando o aparelho já está pareado", async () => {
        const user = userEvent.setup();
        serverWith([integracao({ id: "int_1", connection_state: "connected" })], {
            connected: true,
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        expect(await screen.findByText("Aparelho conectado")).toBeInTheDocument();
        expect(
            screen.queryByRole("img", { name: "QR de pareamento do WhatsApp" }),
        ).not.toBeInTheDocument();
    });

    it("explica o 409 whatsapp_nao_configurado sem quebrar a aba", async () => {
        const user = userEvent.setup();
        serverWith([integracao({ id: "int_1", connection_state: "disconnected" })], {
            onQrcode: () => ({
                status: 409,
                corpo: {
                    code: "whatsapp_nao_configurado",
                    message:
                        "esta integração não tem instância da Evolution declarada na configuração do servidor",
                    details: { integration_id: "int_1" },
                },
            }),
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        expect(
            await screen.findByText(/não tem instância da Evolution declarada/),
        ).toBeInTheDocument();
        expect(
            screen.getByText(/ainda não tem uma instância da Evolution declarada/),
        ).toBeInTheDocument();
        expect(screen.getByText("código: whatsapp_nao_configurado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Verificar estado" })).toBeInTheDocument();
        expect(screen.getAllByText("whatsapp").length).toBeGreaterThan(0);
    });

    it("mostra o 503 evolution_indisponivel com a dica e mantém a aba usável", async () => {
        const user = userEvent.setup();
        serverWith([integracao({ id: "int_1", connection_state: "disconnected" })], {
            onState: () => ({
                status: 503,
                corpo: {
                    code: "evolution_indisponivel",
                    message:
                        "o servidor não tem como falar com a Evolution: falta `EVOLUTION_API_KEY` no ambiente",
                    details: null,
                },
            }),
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Verificar estado" }));

        expect(await screen.findByText(/falta `EVOLUTION\_API\_KEY` no ambiente/)).toBeInTheDocument();
        expect(
            screen.getByText(/está sem a chave da Evolution ou o serviço está fora do ar/),
        ).toBeInTheDocument();
        expect(screen.getByText("código: evolution_indisponivel")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desconectar aparelho" })).toBeInTheDocument();
    });

    it("atualiza o estado ao vivo ao verificar e mostra o valor na lista", async () => {
        const user = userEvent.setup();
        const { estado, servidor } = serverWith([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        build();

        expect((await screen.findAllByText("Desconectado")).length).toBeGreaterThan(0);
        estado.opened = true;
        await user.click(screen.getByRole("button", { name: "Verificar estado" }));

        await waitFor(() =>
            expect(screen.getAllByText("Conectado").length).toBeGreaterThan(0),
        );
        expect(screen.getByText("Estado ao vivo: conectado.")).toBeInTheDocument();
        expect(servidor.callsFor("GET", "/whatsapp/state")).toHaveLength(1);
    });

    it("desconecta com confirmação, manda If-Match e devolve o foco ao disparador", async () => {
        const user = userEvent.setup();
        const { servidor } = serverWith([integracao({ id: "int_1", version: 3 })], {
            connected: true,
        });
        build();

        const disparador = await screen.findByRole("button", { name: "Desconectar aparelho" });
        await user.click(disparador);
        await screen.findByRole("dialog");
        await waitForModalFocus();

        await user.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        expect(disparador).toHaveFocus();

        await user.click(disparador);
        const confirmation = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.click(within(confirmation).getByRole("button", { name: "Desconectar" }));

        expect(
            await screen.findByText(/Aparelho desconectado\. A integração continua cadastrada/),
        ).toBeInTheDocument();
        const calls = servidor.callsFor("POST", "/whatsapp/disconnect");
        expect(calls).toHaveLength(1);
        expect(calls[0]!.headers["if-match"]).toBe("3");
        expect(screen.getAllByText("Desconectado").length).toBeGreaterThan(0);
        expect(
            screen.queryByRole("img", { name: "QR de pareamento do WhatsApp" }),
        ).not.toBeInTheDocument();
    });

    it("no 412 do desconectar relê a versão e mantém o painel usável", async () => {
        const user = userEvent.setup();
        let first = true;
        const { servidor, estado } = serverWith([integracao({ id: "int_1", version: 1 })], {
            connected: true,
            onDisconnect: () => {
                if (first) {
                    first = false;
                    estado.itens = estado.itens.map((item) =>
                        item.id === "int_1" ? { ...item, version: 5 } : item,
                    );
                    return {
                        status: 412,
                        corpo: {
                            code: "version_conflict",
                            message: "conflito de versão",
                            details: { current_version: 5, expected_version: 1 },
                        },
                    };
                }
                estado.opened = false;
                estado.itens = estado.itens.map((item) =>
                    item.id === "int_1"
                        ? { ...item, connection_state: "disconnected", version: 6 }
                        : item,
                );
                return {
                    status: 200,
                    corpo: {
                        integration_id: "int_1",
                        instance: "loja",
                        connection_state: "disconnected",
                        disconnected: true,
                    },
                };
            },
        });
        build();

        await user.click(await screen.findByRole("button", { name: "Desconectar aparelho" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.click(within(modal).getByRole("button", { name: "Desconectar" }));

        expect(await screen.findByText(/Outra pessoa alterou esta integração/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 5/)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Desconectar aparelho" }));
        const second = await screen.findByRole("dialog");
        await waitForModalFocus();
        await user.click(within(second).getByRole("button", { name: "Desconectar" }));

        await waitFor(() =>
            expect(servidor.callsFor("POST", "/whatsapp/disconnect")).toHaveLength(2),
        );
        expect(
            servidor.callsFor("POST", "/whatsapp/disconnect")[1]!.headers["if-match"],
        ).toBe("5");
    });

    it("traduz tabela, painel do WhatsApp e modal ao alternar para en-US", async () => {
        const user = userEvent.setup();
        serverWith([integracao({ id: "int_1", kind: "whatsapp", connection_state: "disconnected" })]);
        build();

        expect(await screen.findByRole("table", { name: "Integrações do agente" })).toBeInTheDocument();

        act(() => {
            setLocale("en-US");
        });

        expect(await screen.findByRole("table", { name: "Agent integrations" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New integration" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Connect / generate QR" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Check state" })).toBeInTheDocument();
        expect(screen.getByText("1 integration")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "New integration" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        expect(within(modal).getByText("New integration")).toBeInTheDocument();
        expect(within(modal).getByRole("button", { name: "Create integration" })).toBeInTheDocument();
        setLocale("pt-BR");
    });
});
