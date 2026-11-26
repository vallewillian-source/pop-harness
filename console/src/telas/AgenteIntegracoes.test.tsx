/**
 * Testes da aba Integrações do agente (HR-44 e HR-44d).
 *
 * O que importa: listar as integrações com situação, estado de conexão e **segredo só indicado**
 * (nunca o valor), criar enviando apenas o corpo que a API aceita (`kind` + `status`), recusar
 * formulário vazio sem chamar a API, alterar a situação com `If-Match` e, no `412`, reler a versão
 * e manter o que o operador escolheu, mostrar o motivo real da API e excluir com confirmação.
 *
 * Na HR-44d entrou o painel do WhatsApp: QR desenhado do `base64` com rodada e código de
 * pareamento, “já conectado” sem QR, `409 whatsapp_nao_configurado` explicado, estado ao vivo,
 * desconexão com confirmação e `412` no desconectar relendo a versão.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardaAlteracoesProvider } from "../componentes/GuardaAlteracoes";
import { I18nProvider, setLocale } from "../i18n";

afterEach(() => {
    setLocale("pt-BR");
});
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { AgenteIntegracoes } from "./AgenteIntegracoes";

const ORG = "org_1";
const AGENTE = "ag_1";
const LISTA = `/agents/${AGENTE}/integrations`;

const AGENTE_JSON = {
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

function integracao(parcial: Partial<Integracao> & { id: string }): Integracao {
    return {
        kind: "whatsapp",
        status: "active",
        connection_state: "connected",
        version: 1,
        tem_segredo: false,
        configuracao: null,
        segredos_preenchidos: [],
        ...parcial,
    };
}

/**
 * Manifesto do plugin eco, copiado do que `crates/poph-plugin-eco/src/main.rs` declara (HR-44c).
 * É o esquema real usado na prova de ponta a ponta; no teste ele chega pelo `GET …/manifests/eco`.
 */
const MANIFESTO_ECO = {
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

/** Manifesto com segredo **opcional**, para provar que segredo em branco não vai no corpo. */
const MANIFESTO_SMS = {
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

const MANIFESTOS: Record<string, unknown> = { eco: MANIFESTO_ECO, sms: MANIFESTO_SMS };

function montar() {
    return render(
        <I18nProvider>
            <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/integrations`]}>
                <GuardaAlteracoesProvider>
                    <Routes>
                        <Route
                            path="/orgs/:orgId/agents/:agentId/integrations"
                            element={<AgenteIntegracoes />}
                        />
                        <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                    </Routes>
                </GuardaAlteracoesProvider>
            </MemoryRouter>
        </I18nProvider>,
    );
}

/** Abre o menu "⋯" da linha e escolhe a ação (as ações por linha vivem no `SofaMenu`). */
async function escolherNoMenu(
    usuario: ReturnType<typeof userEvent.setup>,
    gatilho: string,
    acao: string,
) {
    await usuario.click(await screen.findByRole("button", { name: gatilho }));
    await usuario.click(await screen.findByRole("menuitem", { name: acao }));
}

/**
 * Servidor falso com as integrações dadas. As rotas de item vêm antes da lista, porque a lista
 * também casa por prefixo.
 */function servidorCom(
    inicial: Integracao[],
    opcoes: {
        aoCriar?: () => { status: number; corpo?: unknown };
        aoAlterar?: () => { status: number; corpo?: unknown };
        aoQrcode?: () => { status: number; corpo?: unknown };
        aoEstado?: () => { status: number; corpo?: unknown };
        aoDesconectar?: (chamada: { cabecalhos: Record<string, string> }) => {
            status: number;
            corpo?: unknown;
        };
        /** Resposta do manifesto por tipo; o padrão é 404 para qualquer tipo. */
        aoManifesto?: (kind: string) => { status: number; corpo?: unknown } | undefined;
        /** Já existe aparelho conectado na instância falsa. */
        conectado?: boolean;
    } = {},
) {
    const estado = { itens: [...inicial], aberto: opcoes.conectado ?? false, pareamentos: 0 };
    const idDoCaminho = (url: string) => url.split(`${LISTA}/`)[1]?.split("/")[0] ?? "";
    const servidor = new FakeServer()
        // Manifesto antes da rota da org: o caminho do manifesto contém `/admin/v1/orgs/<org>`.
        .responder("GET", "/manifests/", (chamada) => {
            const kind = chamada.url.split("/manifests/")[1] ?? "";
            const propria = opcoes.aoManifesto?.(kind);
            if (propria) return propria;
            const manifesto = MANIFESTOS[kind];
            return manifesto
                ? { status: 200, corpo: manifesto }
                : apiError(404, "not_found", `manifesto \`${kind}\` não encontrado`);
        })
        // Rotas de WhatsApp vêm primeiro: os caminhos contêm `${LISTA}` e casariam com as genéricas.
        .responder("POST", "/whatsapp/qrcode", (chamada) => {
            if (opcoes.aoQrcode) return opcoes.aoQrcode();
            const id = idDoCaminho(chamada.url);
            const atual = estado.itens.find((item) => item.id === id);
            if (!atual) return apiError(404, "not_found", "integração não encontrada");
            if (estado.aberto) {
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
            estado.pareamentos += 1;
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
                        base64: `data:image/png;base64,QR-FALSO-${estado.pareamentos}`,
                        code: "2@codigo-falso",
                        count: estado.pareamentos,
                    },
                    expira_em: null,
                    pairing_code: "ABCD-1234",
                },
            };
        })
        .responder("GET", "/whatsapp/state", (chamada) => {
            if (opcoes.aoEstado) return opcoes.aoEstado();
            const id = idDoCaminho(chamada.url);
            const connection_state = estado.aberto ? "connected" : "disconnected";
            estado.itens = estado.itens.map((item) =>
                item.id === id ? { ...item, connection_state } : item,
            );
            return {
                status: 200,
                corpo: {
                    integration_id: id,
                    instance: "loja",
                    state: estado.aberto ? "open" : "close",
                    connection_state,
                },
            };
        })
        .responder("POST", "/whatsapp/disconnect", (chamada) => {
            if (opcoes.aoDesconectar) return opcoes.aoDesconectar(chamada);
            const id = idDoCaminho(chamada.url);
            estado.aberto = false;
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
        .responder("PATCH", `${LISTA}/`, (chamada) => {
            if (opcoes.aoAlterar) return opcoes.aoAlterar();
            const id = chamada.url.split(`${LISTA}/`)[1] ?? "";
            const corpo = chamada.corpo as {
                status?: string;
                config_json?: Record<string, unknown>;
                secrets?: Record<string, string>;
            };
            const atual = estado.itens.find((item) => item.id === id);
            if (!atual) return apiError(404, "not_found", "integração não encontrada");
            const preenchidos = corpo.secrets
                ? [
                      ...new Set([
                          ...(atual.segredos_preenchidos ?? []),
                          ...Object.keys(corpo.secrets),
                      ]),
                  ].filter((nome) => (corpo.secrets![nome] ?? "x") !== "")
                : (atual.segredos_preenchidos ?? []);
            const alterada = {
                ...atual,
                status: corpo.status ?? atual.status,
                configuracao: corpo.config_json ?? atual.configuracao ?? null,
                segredos_preenchidos: preenchidos,
                tem_segredo: preenchidos.length > 0 || atual.tem_segredo,
                version: atual.version + 1,
            };
            estado.itens = estado.itens.map((item) => (item.id === id ? alterada : item));
            return { status: 200, corpo: alterada };
        })
        .responder("GET", `${LISTA}/`, (chamada) => {
            const id = chamada.url.split(`${LISTA}/`)[1] ?? "";
            const achada = estado.itens.find((item) => item.id === id);
            return achada
                ? { status: 200, corpo: achada }
                : apiError(404, "not_found", "integração não encontrada");
        })
        .responder("DELETE", `${LISTA}/`, (chamada) => {
            const id = chamada.url.split(`${LISTA}/`)[1] ?? "";
            estado.itens = estado.itens.filter((item) => item.id !== id);
            return { status: 200, corpo: { deleted: true } };
        })
        .responder("POST", LISTA, (chamada) => {
            if (opcoes.aoCriar) return opcoes.aoCriar();
            const corpo = chamada.corpo as {
                kind: string;
                status?: string;
                config_json?: Record<string, unknown>;
                secrets?: Record<string, string>;
            };
            const criada = integracao({
                id: "int_nova",
                kind: corpo.kind,
                status: corpo.status ?? "active",
                connection_state: "disconnected",
                configuracao: corpo.config_json ?? null,
                segredos_preenchidos: corpo.secrets ? Object.keys(corpo.secrets) : [],
                tem_segredo: !!corpo.secrets,
            });
            estado.itens = [...estado.itens, criada];
            return { status: 201, corpo: criada };
        })
        .responder("GET", LISTA, () => ({
            status: 200,
            corpo: { items: estado.itens, next_cursor: null },
        }))
        .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENTE_JSON })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return { servidor: servidor.instalar(), estado };
}

describe("Aba Integrações", () => {
    it("lista tipo, situação, conexão e indica o segredo sem exibir o valor", async () => {
        servidorCom([
            integracao({ id: "int_1", tem_segredo: true, connection_state: "connected" }),
            integracao({
                id: "int_2",
                kind: "eco",
                status: "inactive",
                connection_state: "awaiting_scan",
            }),
        ]);
        montar();

        expect(await screen.findByText("whatsapp")).toBeInTheDocument();
        expect(screen.getByText("eco")).toBeInTheDocument();
        expect(screen.getAllByText("Ativa").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Inativa").length).toBeGreaterThan(0);
        expect(screen.getByText("gravado (não exibido)")).toBeInTheDocument();
        expect(screen.getByText("sem segredo")).toBeInTheDocument();
        // O painel do WhatsApp mostra o estado que o canal informou.
        expect(screen.getAllByText("Conectado").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Aguardando leitura do QR").length).toBeGreaterThan(0);
        // O painel do WhatsApp agora oferece o pareamento e a desconexão de verdade.
        expect(screen.getByText("Configuração pelo formulário do plugin")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desconectar aparelho" })).toBeInTheDocument();
        expect(screen.getByText("WhatsApp: pareamento e conexão")).toBeInTheDocument();

        // RL-12: lista plana (tabela contínua) e ações por linha no menu "⋯", sem cartão.
        const tabela = screen.getByRole("table", { name: "Integrações do agente" });
        expect(within(tabela).getByRole("columnheader", { name: "Conexão" })).toBeInTheDocument();
        expect(
            within(tabela).getByRole("button", { name: "Ações de whatsapp (int_1)" }),
        ).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).not.toBeInTheDocument();
    });

    it("cria enviando só tipo e situação e mostra a integração na lista", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([]);
        montar();

        expect(await screen.findByText("Nenhuma integração neste agente")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");
        await usuario.selectOptions(within(modal).getByLabelText(/Situação/), "inactive");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(1));
        const envio = servidor.chamadasPara("POST", LISTA)[0]!;
        expect(envio.corpo).toEqual({ kind: "whatsapp", status: "inactive" });
        expect(await screen.findByText("Integração whatsapp criada.")).toBeInTheDocument();
        expect(await screen.findByText("whatsapp")).toBeInTheDocument();
    });

    it("não chama a API quando o tipo está vazio", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([]);
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(
            await within(modal).findByText("informe o tipo da integração (por exemplo, whatsapp)"),
        ).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(0);
    });

    it("mostra o motivo real da API quando a criação é recusada", async () => {
        const usuario = userEvent.setup();
        servidorCom([], {
            aoCriar: () =>
                apiError(422, "manifest_indisponivel", "configuração exige o manifesto", {
                    field: "kind",
                    reason: "tipo recusado pelo servidor",
                }),
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(await within(modal).findByText("tipo recusado pelo servidor")).toBeInTheDocument();
    });

    it("no 412 relê a versão, mantém a escolha e salva de novo na versão atual", async () => {
        const usuario = userEvent.setup();
        let primeira = true;
        const { servidor, estado } = servidorCom([integracao({ id: "int_1", version: 1 })], {
            aoAlterar: () => {
                if (primeira) {
                    primeira = false;
                    // Outra pessoa salvou: o registro já está na versão 5.
                    estado.itens = estado.itens.map((item) =>
                        item.id === "int_1" ? { ...item, version: 5 } : item,
                    );
                    return apiError(412, "version_conflict", "conflito de versão", {
                        current_version: 5,
                        expected_version: 1,
                    });
                }
                const atual = estado.itens.find((item) => item.id === "int_1")!;
                const alterada = { ...atual, status: "inactive", version: atual.version + 1 };
                estado.itens = estado.itens.map((item) =>
                    item.id === "int_1" ? alterada : item,
                );
                return { status: 200, corpo: alterada };
            },
        });
        montar();

        await escolherNoMenu(usuario, "Ações de whatsapp (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.selectOptions(within(modal).getByLabelText(/Situação/), "inactive");
        await usuario.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        expect(await within(modal).findByText(/Versão desatualizada/)).toBeInTheDocument();
        expect(within(modal).getByText(/agora na versão 5/)).toBeInTheDocument();
        // O que o operador escolheu continua selecionado.
        expect(within(modal).getByLabelText(/Situação/)).toHaveValue("inactive");

        await usuario.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        await waitFor(() =>
            expect(servidor.chamadasPara("PATCH", `${LISTA}/int_1`)).toHaveLength(2),
        );
        const segunda = servidor.chamadasPara("PATCH", `${LISTA}/int_1`)[1]!;
        expect(segunda.cabecalhos["if-match"]).toBe("5");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("exclui com confirmação pelo tipo e sem force", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([integracao({ id: "int_1" })]);
        montar();

        await escolherNoMenu(usuario, "Ações de whatsapp (int_1)", "Excluir");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        const confirmar = within(modal).getByRole("button", { name: "Excluir integração" });
        expect(confirmar).toBeDisabled();

        await usuario.type(within(modal).getByLabelText(/Digite whatsapp/), "whatsapp");
        expect(confirmar).toBeEnabled();
        await usuario.click(confirmar);

        await waitFor(() =>
            expect(servidor.chamadasPara("DELETE", `${LISTA}/int_1`)).toHaveLength(1),
        );
        const exclusao = servidor.chamadasPara("DELETE", `${LISTA}/int_1`)[0]!;
        expect(exclusao.url).not.toContain("force");
        expect(
            await screen.findByText("Nenhuma integração neste agente"),
        ).toBeInTheDocument();
    });
});

describe("Formulário do manifesto na integração (HR-44e)", () => {
    it("desenha o formulário do tipo a partir do manifesto, com obrigatório e segredo mascarado", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "eco");

        // O formulário vem do manifesto do plugin: nenhum campo é escrito à mão no Console.
        const remetente = await within(modal).findByLabelText(/Nome do remetente/);
        expect(remetente).toHaveValue("");
        expect(within(modal).getByText(/Configuração do plugin “Eco \(exemplo\)”/)).toBeInTheDocument();
        const token = within(modal).getByLabelText(/Token do canal/);
        expect(token).toHaveAttribute("type", "password");
        expect(token).toHaveValue("");
        expect(within(modal).getByLabelText(/Modo/)).toHaveValue("inverter");
        // O rótulo do campo obrigatório vem marcado.
        expect(remetente).toBeRequired();
    });

    it("tipo sem manifesto avisa e cria só com tipo e situação", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([]);
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "whatsapp");

        expect(await within(modal).findByText("Este tipo não tem formulário")).toBeInTheDocument();
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(1));
        expect(servidor.chamadasPara("POST", LISTA)[0]!.corpo).toEqual({
            kind: "whatsapp",
            status: "active",
        });
    });

    it("envia config_json com os tipos do esquema e não manda segredo em branco", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([]);
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "sms");
        await usuario.type(await within(modal).findByLabelText(/Remetente/), "5511999999999");
        await usuario.clear(within(modal).getByLabelText(/Tentativas/));
        await usuario.type(within(modal).getByLabelText(/Tentativas/), "4");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(1));
        const envio = servidor.chamadasPara("POST", LISTA)[0]!;
        // `tentativas` vai como número (o servidor cobra o tipo do esquema); o segredo em branco
        // fica fora do corpo, porque branco significa "manter o que está gravado".
        expect(envio.corpo).toEqual({
            kind: "sms",
            status: "active",
            config_json: { remetente: "5511999999999", tentativas: 4 },
        });
    });

    it("manda o segredo digitado e o obrigatório é cobrado antes de chamar a API", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([]);
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "eco");
        await usuario.type(await within(modal).findByLabelText(/Nome do remetente/), "Loja");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(await within(modal).findByText("informe o segredo")).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(0);

        await usuario.type(within(modal).getByLabelText(/Token do canal/), "segredo-do-teste");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", LISTA)).toHaveLength(1));
        const envio = servidor.chamadasPara("POST", LISTA)[0]!;
        expect(envio.corpo).toEqual({
            kind: "eco",
            status: "active",
            config_json: { modo: "inverter", remetente: "Loja" },
            secrets: { token: "segredo-do-teste" },
        });
        // O valor digitado não fica escrito na tela depois de salvar.
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        expect(screen.queryByText("segredo-do-teste")).not.toBeInTheDocument();
    });

    it("põe o erro de validação da API no campo que ela aponta", async () => {
        const usuario = userEvent.setup();
        servidorCom([], {
            aoCriar: () =>
                apiError(400, "validation_error", "configuração inválida", {
                    field: "config_json.remetente",
                    reason: "precisa de pelo menos 3 caractere(s)",
                }),
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "eco");
        await usuario.type(await within(modal).findByLabelText(/Nome do remetente/), "Loja");
        await usuario.type(within(modal).getByLabelText(/Token do canal/), "chave");
        await usuario.click(within(modal).getByRole("button", { name: "Criar integração" }));

        expect(
            await within(modal).findByText("precisa de pelo menos 3 caractere(s)"),
        ).toBeInTheDocument();
        // O diálogo continua aberto, com o que foi digitado.
        expect(within(modal).getByLabelText(/Nome do remetente/)).toHaveValue("Loja");
    });

    it("no 412 relê a versão e mantém configuração e segredo digitados nesta sessão", async () => {
        const usuario = userEvent.setup();
        let primeira = true;
        const { servidor, estado } = servidorCom(
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
                aoAlterar: () => {
                    if (primeira) {
                        primeira = false;
                        estado.itens = estado.itens.map((item) =>
                            item.id === "int_1" ? { ...item, version: 5 } : item,
                        );
                        return apiError(412, "version_conflict", "conflito de versão", {
                            current_version: 5,
                            expected_version: 1,
                        });
                    }
                    const atual = estado.itens.find((item) => item.id === "int_1")!;
                    const alterada = { ...atual, version: atual.version + 1 };
                    estado.itens = estado.itens.map((item) =>
                        item.id === "int_1" ? alterada : item,
                    );
                    return { status: 200, corpo: alterada };
                },
            },
        );
        montar();

        await escolherNoMenu(usuario, "Ações de eco (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        const remetente = await within(modal).findByLabelText(/Nome do remetente/);
        // O formulário já vem preenchido com a configuração gravada.
        expect(remetente).toHaveValue("Antigo");
        await usuario.clear(remetente);
        await usuario.type(remetente, "Novo remetente");
        await usuario.type(within(modal).getByLabelText(/Token do canal/), "chave-nova");
        await usuario.click(within(modal).getByRole("button", { name: "Salvar alterações" }));

        expect(await within(modal).findByText(/Versão desatualizada/)).toBeInTheDocument();
        expect(within(modal).getByText(/agora na versão 5/)).toBeInTheDocument();
        // Nem a configuração nem o segredo digitado se perdem no conflito.
        expect(within(modal).getByLabelText(/Nome do remetente/)).toHaveValue("Novo remetente");
        expect(within(modal).getByLabelText(/Token do canal/)).toHaveValue("chave-nova");

        await usuario.click(within(modal).getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() =>
            expect(servidor.chamadasPara("PATCH", `${LISTA}/int_1`)).toHaveLength(2),
        );
        const segunda = servidor.chamadasPara("PATCH", `${LISTA}/int_1`)[1]!;
        expect(segunda.cabecalhos["if-match"]).toBe("5");
        expect(segunda.corpo).toEqual({
            status: "active",
            config_json: { modo: "inverter", remetente: "Novo remetente" },
            secrets: { token: "chave-nova" },
        });
    });

    it("segredos_preenchidos null diz que há segredo ilegível e cobra regravar", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([
            integracao({
                id: "int_1",
                kind: "eco",
                tem_segredo: true,
                configuracao: { modo: "inverter", remetente: "Loja" },
                segredos_preenchidos: null,
            }),
        ]);
        montar();

        await escolherNoMenu(usuario, "Ações de eco (int_1)", "Editar");
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();

        expect(
            await within(modal).findByText(/a chave de criptografia atual do servidor não consegue lê-lo/),
        ).toBeInTheDocument();
        expect(within(modal).getByText(/A tela não sabe qual segredo está gravado/)).toBeInTheDocument();

        await usuario.click(within(modal).getByRole("button", { name: "Salvar alterações" }));
        expect(await within(modal).findByText("informe o segredo")).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `${LISTA}/int_1`)).toHaveLength(0);
    });

    it("plugin que não respondeu ao handshake explica e não deixa salvar", async () => {
        const usuario = userEvent.setup();
        servidorCom([], {
            aoManifesto: () => ({
                status: 503,
                corpo: {
                    code: "manifesto_indisponivel",
                    message: "o plugin deste tipo não respondeu ao handshake",
                    details: { kind: "eco" },
                },
            }),
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Criar primeira integração" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.type(within(modal).getByLabelText(/Tipo da integração/), "eco");

        expect(
            await within(modal).findByText(/não respondeu ao handshake/),
        ).toBeInTheDocument();
        expect(within(modal).getByText(/código: manifesto_indisponivel/)).toBeInTheDocument();
        expect(within(modal).getByRole("button", { name: "Criar integração" })).toBeDisabled();
    });
});

describe("Painel do WhatsApp (HR-44d)", () => {
    it("gera o QR com If-Match, desenha o base64 e mostra a rodada e o código de pareamento", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        const imagem = await screen.findByRole("img", { name: "QR de pareamento do WhatsApp" });
        expect(imagem).toHaveAttribute("src", "data:image/png;base64,QR-FALSO-1");
        expect(screen.getByText("ABCD-1234")).toBeInTheDocument();
        expect(screen.getByText("rodada 1 do QR")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Gerar outro QR" })).toBeInTheDocument();

        const chamadas = servidor.chamadasPara("POST", "/whatsapp/qrcode");
        expect(chamadas).toHaveLength(1);
        expect(chamadas[0]!.cabecalhos["if-match"]).toBe("1");
        // Sem número: o pareamento é pelo QR, e o corpo não é enviado.
        expect(chamadas[0]!.bruto).toBeUndefined();
    });

    it("pareia por número quando o operador digita e recusa número fora do formato", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        montar();

        const campo = await screen.findByLabelText(/Número do WhatsApp/);
        await usuario.type(campo, "119");
        await usuario.click(screen.getByRole("button", { name: "Conectar / gerar QR" }));
        expect(
            await screen.findByText("use só dígitos, com DDI e DDD (ex.: 5511999998888)"),
        ).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/whatsapp/qrcode")).toHaveLength(0);

        await usuario.clear(campo);
        await usuario.type(campo, "5511999998888");
        await usuario.click(screen.getByRole("button", { name: "Conectar / gerar QR" }));
        await waitFor(() =>
            expect(servidor.chamadasPara("POST", "/whatsapp/qrcode")).toHaveLength(1),
        );
        expect(servidor.chamadasPara("POST", "/whatsapp/qrcode")[0]!.corpo).toEqual({
            number: "5511999998888",
        });
    });

    it("mostra “aparelho conectado” sem QR quando o aparelho já está pareado", async () => {
        const usuario = userEvent.setup();
        servidorCom([integracao({ id: "int_1", connection_state: "connected" })], {
            conectado: true,
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        expect(await screen.findByText("Aparelho conectado")).toBeInTheDocument();
        expect(
            screen.queryByRole("img", { name: "QR de pareamento do WhatsApp" }),
        ).not.toBeInTheDocument();
    });

    it("explica o 409 whatsapp_nao_configurado sem quebrar a aba", async () => {
        const usuario = userEvent.setup();
        servidorCom([integracao({ id: "int_1", connection_state: "disconnected" })], {
            aoQrcode: () => ({
                status: 409,
                corpo: {
                    code: "whatsapp_nao_configurado",
                    message:
                        "esta integração não tem instância da Evolution declarada na configuração do servidor",
                    details: { integration_id: "int_1" },
                },
            }),
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Conectar / gerar QR" }));

        expect(
            await screen.findByText(/não tem instância da Evolution declarada/),
        ).toBeInTheDocument();
        expect(
            screen.getByText(/ainda não tem uma instância da Evolution declarada/),
        ).toBeInTheDocument();
        expect(screen.getByText("código: whatsapp_nao_configurado")).toBeInTheDocument();
        // A aba continua utilizável: a lista e as ações seguem na tela.
        expect(screen.getByRole("button", { name: "Verificar estado" })).toBeInTheDocument();
        expect(screen.getAllByText("whatsapp").length).toBeGreaterThan(0);
    });

    it("mostra o 503 evolution_indisponivel com a dica e mantém a aba usável", async () => {
        const usuario = userEvent.setup();
        servidorCom([integracao({ id: "int_1", connection_state: "disconnected" })], {
            aoEstado: () => ({
                status: 503,
                corpo: {
                    code: "evolution_indisponivel",
                    message:
                        "o servidor não tem como falar com a Evolution: falta `EVOLUTION_API_KEY` no ambiente",
                    details: null,
                },
            }),
        });
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Verificar estado" }));

        expect(await screen.findByText(/falta `EVOLUTION\_API\_KEY` no ambiente/)).toBeInTheDocument();
        expect(
            screen.getByText(/está sem a chave da Evolution ou o serviço está fora do ar/),
        ).toBeInTheDocument();
        expect(screen.getByText("código: evolution_indisponivel")).toBeInTheDocument();
        // A aba continua utilizável: outras ações seguem disponíveis.
        expect(screen.getByRole("button", { name: "Desconectar aparelho" })).toBeInTheDocument();
    });

    it("atualiza o estado ao vivo ao verificar e mostra o valor na lista", async () => {
        const usuario = userEvent.setup();
        const { estado, servidor } = servidorCom([
            integracao({ id: "int_1", connection_state: "disconnected" }),
        ]);
        montar();

        expect((await screen.findAllByText("Desconectado")).length).toBeGreaterThan(0);
        estado.aberto = true;
        await usuario.click(screen.getByRole("button", { name: "Verificar estado" }));

        await waitFor(() =>
            expect(screen.getAllByText("Conectado").length).toBeGreaterThan(0),
        );
        expect(screen.getByText("Estado ao vivo: conectado.")).toBeInTheDocument();
        expect(servidor.chamadasPara("GET", "/whatsapp/state")).toHaveLength(1);
    });

    it("desconecta com confirmação, manda If-Match e devolve o foco ao disparador", async () => {
        const usuario = userEvent.setup();
        const { servidor } = servidorCom([integracao({ id: "int_1", version: 3 })], {
            conectado: true,
        });
        montar();

        const disparador = await screen.findByRole("button", { name: "Desconectar aparelho" });
        await usuario.click(disparador);
        await screen.findByRole("dialog");
        await waitForModalFocus();

        // Esc fecha e devolve o foco ao botão que abriu o diálogo.
        await usuario.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        expect(disparador).toHaveFocus();

        await usuario.click(disparador);
        const confirmacao = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.click(within(confirmacao).getByRole("button", { name: "Desconectar" }));

        expect(
            await screen.findByText(/Aparelho desconectado\. A integração continua cadastrada/),
        ).toBeInTheDocument();
        const chamadas = servidor.chamadasPara("POST", "/whatsapp/disconnect");
        expect(chamadas).toHaveLength(1);
        expect(chamadas[0]!.cabecalhos["if-match"]).toBe("3");
        expect(screen.getAllByText("Desconectado").length).toBeGreaterThan(0);
        // Sem QR na tela depois de desconectar.
        expect(
            screen.queryByRole("img", { name: "QR de pareamento do WhatsApp" }),
        ).not.toBeInTheDocument();
    });

    it("no 412 do desconectar relê a versão e mantém o painel usável", async () => {
        const usuario = userEvent.setup();
        let primeira = true;
        const { servidor, estado } = servidorCom([integracao({ id: "int_1", version: 1 })], {
            conectado: true,
            aoDesconectar: () => {
                if (primeira) {
                    primeira = false;
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
                estado.aberto = false;
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
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Desconectar aparelho" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.click(within(modal).getByRole("button", { name: "Desconectar" }));

        expect(await screen.findByText(/Outra pessoa alterou esta integração/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 5/)).toBeInTheDocument();

        // O painel continua utilizável e a próxima tentativa usa a versão relida.
        await usuario.click(screen.getByRole("button", { name: "Desconectar aparelho" }));
        const segunda = await screen.findByRole("dialog");
        await waitForModalFocus();
        await usuario.click(within(segunda).getByRole("button", { name: "Desconectar" }));

        await waitFor(() =>
            expect(servidor.chamadasPara("POST", "/whatsapp/disconnect")).toHaveLength(2),
        );
        expect(
            servidor.chamadasPara("POST", "/whatsapp/disconnect")[1]!.cabecalhos["if-match"],
        ).toBe("5");
    });

    it("traduz tabela, painel do WhatsApp e modal ao alternar para en-US", async () => {
        const usuario = userEvent.setup();
        servidorCom([integracao({ id: "int_1", kind: "whatsapp", connection_state: "disconnected" })]);
        montar();

        expect(await screen.findByRole("table", { name: "Integrações do agente" })).toBeInTheDocument();

        act(() => {
            setLocale("en-US");
        });

        expect(await screen.findByRole("table", { name: "Agent integrations" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New integration" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Connect / generate QR" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Check state" })).toBeInTheDocument();
        expect(screen.getByText("1 integration")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "New integration" }));
        const modal = await screen.findByRole("dialog");
        await waitForModalFocus();
        expect(within(modal).getByText("New integration")).toBeInTheDocument();
        expect(within(modal).getByRole("button", { name: "Create integration" })).toBeInTheDocument();
        setLocale("pt-BR");
    });
});
