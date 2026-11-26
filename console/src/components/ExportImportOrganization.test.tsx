
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FakeServer, apiError } from "../testing/server";
import { ExportImportOrganization } from "../components/ExportImportOrganization";
import { LIMIT_IMPORT_BYTES } from "../api/backup";
import { setLocale } from "../i18n";

const ORG = "org_1";
const NAME_FILE = "org-origem-backup-20261121-1118.json";
const CONTENT_FILE = JSON.stringify({ formato: "poph-backup-org", versao_formato: 1 });

const REPORT = {
    org: { id: "org_1", criada: false },
    com_conversas: false,
    tabelas: {
        agents: { inseridas: 2, atualizadas: 0 },
        products: { inseridas: 0, atualizadas: 3 },
        departments: { inseridas: 0, atualizadas: 0 },
    },
    total: 5,
    segredos_a_redigitar: 1,
    indice: { base_conhecimento: { agentes: 1, trechos: 4 }, produtos: { trechos: 2 } },
    aviso: null,
};

const headerAttachment = { "content-disposition": `attachment; filename="${NAME_FILE}"` };

let clicks: string[] = [];
let createUrl: ReturnType<typeof vi.fn>;
let revokeUrl: ReturnType<typeof vi.fn>;
const urlOriginal = { criar: URL.createObjectURL, revoke: URL.revokeObjectURL };

beforeEach(() => {
    clicks = [];
    createUrl = vi.fn(() => "blob:teste");
    revokeUrl = vi.fn();
    URL.createObjectURL = createUrl as unknown as typeof URL.createObjectURL;
    URL.revokeObjectURL = revokeUrl as unknown as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
        clicks.push(this.download);
    });
});

afterEach(() => {
    URL.createObjectURL = urlOriginal.criar;
    URL.revokeObjectURL = urlOriginal.revoke;
});

function build() {
    return render(<ExportImportOrganization org={ORG} />);
}

function fileJson(content = CONTENT_FILE, nome = "backup.json", size?: number): File {
    const file = new File([content], nome, { type: "application/json" });
    if (size !== undefined) {
        Object.defineProperty(file, "size", { value: size });
    }
    return file;
}

function fieldFile(): HTMLInputElement {
    const campo = document.querySelector('input[type="file"]');
    if (!(campo instanceof HTMLInputElement)) throw new Error("campo de arquivo ausente");
    return campo;
}

async function choose(file: File) {
    await userEvent.upload(fieldFile(), file);
}

async function confirmImport() {
    await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Importar agora" }));
}

describe("backup em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("mostra a seção, o relatório e os conflitos em inglês", async () => {
        setLocale("en-US");
        new FakeServer()
            .respond("GET", "/backup", {
                status: 200,
                texto: CONTENT_FILE,
                headers: headerAttachment,
            })
            .respond("POST", "/backup/import", { status: 200, corpo: REPORT })
            .install();
        build();

        expect(screen.getByRole("heading", { name: "Export and import the organization" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Download organization file" })).toBeInTheDocument();
        expect(screen.getByRole("checkbox", { name: /Include conversations/ })).toBeInTheDocument();

        await userEvent.click(screen.getByRole("button", { name: "Download organization file" }));
        expect(await screen.findByText(`File "${NAME_FILE}" downloaded.`)).toBeInTheDocument();

        await choose(fileJson());
        await userEvent.click(screen.getByRole("button", { name: /Import file into this organization/ }));
        const dialog = await screen.findByRole("dialog");
        await userEvent.click(within(dialog).getByRole("button", { name: "Import now" }));

        expect(await screen.findByText(/5 rows written: 2 new and 3 updated/)).toBeInTheDocument();
        expect(screen.getByText("Secrets to type again")).toBeInTheDocument();
        expect(screen.getByText(/1 integration arrived without the secret/)).toBeInTheDocument();
        expect(screen.getByText("Search ready")).toBeInTheDocument();
        const table = screen.getByRole("table", { name: "Rows written per table" });
        expect(within(table).getByText("Agents")).toBeInTheDocument();
        expect(within(table).getByText("Products")).toBeInTheDocument();
    });

    it("traduz a recusa de sobrescrita e os nomes das tabelas", async () => {
        setLocale("en-US");
        new FakeServer()
            .respond("POST", "/backup/import", {
                status: 409,
                corpo: {
                    code: "importacao_sobrescreve",
                    message: "cru",
                    details: {
                        tabelas: { agents: 1, products: 3 },
                        linhas: 4,
                        exemplos: ["agents: bia", "products: dedetizacao"],
                    },
                },
            })
            .install();
        build();

        await choose(fileJson());
        await userEvent.click(screen.getByRole("button", { name: /Import file into this organization/ }));
        const dialog = await screen.findByRole("dialog");
        await userEvent.click(within(dialog).getByRole("button", { name: "Import now" }));

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("This import will overwrite what already exists")).toBeInTheDocument();
        expect(within(alert).getByText(/4 rows in this organization would be updated/)).toBeInTheDocument();
        const table = within(alert).getByRole("table", { name: "Rows that would be overwritten per table" });
        expect(within(table).getByText("Agents")).toBeInTheDocument();
        const list = within(alert).getByRole("list", { name: "Examples of what would be overwritten" });
        const items = within(list).getAllByRole("listitem");
        expect(items[0]).toHaveTextContent("Agents: bia");
        expect(items[1]).toHaveTextContent("Products: dedetizacao");
        expect(within(alert).getByRole("button", { name: "Overwrite and import" })).toBeEnabled();
    });
});

describe("exportar", () => {
    it("sem conversas: chama a rota sem query e baixa com o nome que o servidor mandou", async () => {
        const servidor = new FakeServer()
            .respond("GET", "/backup", { status: 200, texto: CONTENT_FILE, headers: headerAttachment })
            .install();
        build();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        await waitFor(() => expect(clicks).toEqual([NAME_FILE]));
        const calls = servidor.callsFor("GET", "/backup");
        expect(calls).toHaveLength(1);
        expect(calls[0]?.url).toBe("/admin/v1/orgs/org_1/backup");
        expect(createUrl).toHaveBeenCalledTimes(1);
        expect(screen.getByText(`Arquivo "${NAME_FILE}" baixado.`)).toBeInTheDocument();
    });

    it("com 'Incluir conversas' marcado: manda com_conversas=true", async () => {
        const servidor = new FakeServer()
            .respond("GET", "/backup", { status: 200, texto: CONTENT_FILE, headers: headerAttachment })
            .install();
        build();

        const box = screen.getByRole("checkbox", { name: /Incluir conversas/ });
        expect(box).not.toBeChecked();
        await userEvent.click(box);
        expect(box).toBeChecked();
        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(servidor.callsFor("GET", "/backup")[0]?.url).toBe(
            "/admin/v1/orgs/org_1/backup?com_conversas=true",
        );
    });

    it("409 backup_incompleto explica o que fazer e não oferece tentar de novo", async () => {
        new FakeServer()
            .respond("GET", "/backup", apiError(409, "backup_incompleto", "arquivo sem conteúdo", { name: "catalogo.png" }))
            .install();
        build();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        expect(
            await screen.findByText(/Um arquivo da biblioteca desta organização não está no disco/),
        ).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tentar de novo" })).toBeNull();
        expect(clicks).toHaveLength(0);
    });

    it("erro de servidor no download mostra 'Tentar de novo' e o botão refaz a exportação", async () => {
        let attempt = 0;
        const servidor = new FakeServer()
            .respond("GET", "/backup", () => {
                attempt += 1;
                return attempt === 1
                    ? apiError(500, "interno", "falha")
                    : { status: 200, texto: CONTENT_FILE, headers: headerAttachment };
            })
            .install();
        build();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));
        const alert = await screen.findByRole("alert");
        await userEvent.click(within(alert).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() => expect(clicks).toEqual([NAME_FILE]));
        expect(servidor.callsFor("GET", "/backup")).toHaveLength(2);
    });

    it("não dispara exportação nem importação ao montar a seção", () => {
        const servidor = new FakeServer()
            .respond("GET", "/backup", { status: 200, texto: CONTENT_FILE, headers: headerAttachment })
            .respond("POST", "/backup/import", { status: 200, corpo: REPORT })
            .install();
        build();

        expect(servidor.calls).toHaveLength(0);
    });
});

describe("importar", () => {
    it("sucesso: envia o texto do arquivo como JSON cru e mostra o relatório com segredos e índice", async () => {
        const servidor = new FakeServer().respond("POST", "/backup/import", { status: 200, corpo: REPORT }).install();
        build();

        await choose(fileJson());
        expect(screen.getByText(/Escolhido: backup\.json/)).toBeInTheDocument();
        await confirmImport();
        expect(await screen.findByText(/linhas gravadas: 2 novas e 3 atualizadas/)).toBeInTheDocument();

        const dispatch = servidor.callsFor("POST", "/backup/import")[0];
        expect(dispatch?.url).toBe("/admin/v1/orgs/org_1/backup/import");
        expect(dispatch?.headers["content-type"]).toBe("application/json");
        expect(dispatch?.raw).toBe(CONTENT_FILE);

        const table = screen.getByRole("table", { name: "Linhas gravadas por tabela" });
        expect(within(table).getByText("Agentes")).toBeInTheDocument();
        expect(within(table).getByText("Produtos")).toBeInTheDocument();
        expect(within(table).queryByText("Departamentos")).toBeNull();

        expect(screen.getByText("Segredos para digitar de novo")).toBeInTheDocument();
        expect(screen.getByText(/1 integração chegou sem o segredo/)).toBeInTheDocument();
        expect(screen.getByText(/aba Integrações do agente/)).toBeInTheDocument();
        expect(screen.getByText("Busca pronta")).toBeInTheDocument();
        expect(screen.queryByText("Importado, índice pendente")).toBeNull();
    });

    it("índice sem o modelo: aparece como importado com índice pendente, não como erro", async () => {
        new FakeServer()
            .respond("POST", "/backup/import", {
                status: 200,
                corpo: {
                    ...REPORT,
                    segredos_a_redigitar: 0,
                    indice: { base_conhecimento: { agentes: 0, trechos: 0 }, produtos: { trechos: 0 } },
                    aviso: "modelo de embedding indisponível (model_int8.onnx)",
                },
            })
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        expect(await screen.findByText("Importado, índice pendente")).toBeInTheDocument();
        expect(screen.getByText(/poph kb reindex/)).toBeInTheDocument();
        expect(screen.queryByText(/model_int8/)).toBeNull();
        expect(screen.queryByText("Não consegui importar")).toBeNull();
        expect(screen.queryByText("Segredos para digitar de novo")).toBeNull();
    });

    it("409 conflito: lista até 5 conflitos, diz 'e mais N' e que nada foi gravado", async () => {
        new FakeServer()
            .respond(
                "POST",
                "/backup/import",
                apiError(409, "conflict", "conflito", {
                    conflicts: [
                        "agente bia (slug)",
                        "produto p1 (nome)",
                        "categoria c1 (nome)",
                        "departamento d1 (nome)",
                        "grupo g1 (nome)",
                    ],
                    conflicts_more: 3,
                }),
            )
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("Importação recusada")).toBeInTheDocument();
        expect(within(alert).getByText(/Nada foi gravado/)).toBeInTheDocument();
        const list = within(alert).getByRole("list", { name: "Conflitos encontrados" });
        const items = within(list).getAllByRole("listitem");
        expect(items).toHaveLength(6);
        expect(items[0]).toHaveTextContent("agente bia (slug)");
        expect(items[5]).toHaveTextContent("e mais 3");
        expect(within(alert).queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("400 arquivo de outra versão: mostra as duas versões", async () => {
        new FakeServer()
            .respond(
                "POST",
                "/backup/import",
                apiError(400, "invalid_backup", "versão de formato desconhecida: 99", { version: 99, supported_version: 1 }),
            )
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("Arquivo inválido")).toBeInTheDocument();
        expect(within(alert).getByText(/é da versão 99; esta instalação entende a versão 1/)).toBeInTheDocument();
        expect(within(alert).queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("400 arquivo corrompido: mensagem clara, sem o texto cru da API", async () => {
        new FakeServer()
            .respond("POST", "/backup/import", apiError(400, "invalid_backup", "JSON inválido: EOF at line 1 column 1200"))
            .install();
        build();

        await choose(fileJson("{\"formato\":"));
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText(/não é um backup válido desta instalação/)).toBeInTheDocument();
        expect(alert.textContent).not.toMatch(/EOF/);
    });

    it("413 pelo servidor: diz o limite de 64 MB e que nada foi gravado", async () => {
        new FakeServer().respond("POST", "/backup/import", apiError(413, "payload_too_large", "grande")).install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("Arquivo grande demais")).toBeInTheDocument();
        expect(within(alert).getByText(/passa de 64 MB\. Nada foi gravado/)).toBeInTheDocument();
    });

    it("403 com 'Tentar de novo': o botão pede a confirmação de novo e o segundo envio funciona", async () => {
        let attempt = 0;
        const servidor = new FakeServer()
            .respond("POST", "/backup/import", () => {
                attempt += 1;
                return attempt === 1 ? apiError(403, "forbidden", "sem permissão") : { status: 200, corpo: REPORT };
            })
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText(/não alcança esta organização/)).toBeInTheDocument();
        await userEvent.click(within(alert).getByRole("button", { name: "Tentar de novo" }));

        const dialog = await screen.findByRole("dialog");
        await userEvent.click(within(dialog).getByRole("button", { name: "Importar agora" }));

        expect(await screen.findByText(/linhas gravadas/)).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/backup/import")).toHaveLength(2);
    });

    it("arquivo acima de 64 MB é recusado na tela e nada é enviado", async () => {
        const servidor = new FakeServer().respond("POST", "/backup/import", { status: 200, corpo: REPORT }).install();
        build();

        await choose(fileJson(CONTENT_FILE, "grande.json", LIMIT_IMPORT_BYTES + 1));

        expect(await screen.findByText(/acima do limite de 64 MB/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Importar arquivo nesta organização/ })).toBeDisabled();
        expect(servidor.callsFor("POST", "/backup/import")).toHaveLength(0);
    });

    it("sem arquivo escolhido, o botão de importar fica desligado", () => {
        build();

        expect(screen.getByRole("button", { name: /Importar arquivo nesta organização/ })).toBeDisabled();
    });

    it("Esc e 'Cancelar' fecham a confirmação sem enviar nada", async () => {
        const servidor = new FakeServer().respond("POST", "/backup/import", { status: 200, corpo: REPORT }).install();
        build();

        await choose(fileJson());
        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        await screen.findByRole("dialog");
        await userEvent.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        const dialog = await screen.findByRole("dialog");
        await userEvent.click(within(dialog).getByRole("button", { name: "Cancelar" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        expect(servidor.callsFor("POST", "/backup/import")).toHaveLength(0);
    });

    it("a confirmação explica que importar não apaga o que já existe", async () => {
        build();

        await choose(fileJson());
        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText(/não apaga o que já existe aqui/)).toBeInTheDocument();
    });
});

const DETAIL_OVERWRITES = {
    tabelas: { agents: 1, products: 3 },
    linhas: 4,
    exemplos: ["agents: bia", "products: dedetizacao", "product_categories: Pragas"],
};

describe("recusa de sobrescrita (HR-48e-4)", () => {
    it("(a) recusa mostra o que seria sobrescrito, o total, os exemplos e o botão de autorizar, sem enviar de novo", async () => {
        const servidor = new FakeServer()
            .respond(
                "POST",
                "/backup/import",
                apiError(409, "importacao_sobrescreve", "a importação atualizaria 4 linha(s)", DETAIL_OVERWRITES),
            )
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("Esta importação vai sobrescrever o que já existe")).toBeInTheDocument();
        expect(within(alert).getByText(/4 linhas desta organização seriam atualizadas/)).toBeInTheDocument();
        expect(within(alert).getByText(/Nada foi gravado/)).toBeInTheDocument();

        const table = within(alert).getByRole("table", { name: "Linhas que seriam sobrescritas por tabela" });
        expect(within(table).getByText("Agentes")).toBeInTheDocument();
        expect(within(table).getByText("Produtos")).toBeInTheDocument();

        const list = within(alert).getByRole("list", { name: "Exemplos do que seria sobrescrito" });
        const items = within(list).getAllByRole("listitem");
        expect(items[0]).toHaveTextContent("Agentes: bia");
        expect(items[1]).toHaveTextContent("Produtos: dedetizacao");
        expect(items[2]).toHaveTextContent("Categorias de produto: Pragas");

        expect(within(alert).getByRole("button", { name: "Sobrescrever e importar" })).toBeEnabled();
        const sends = servidor.callsFor("POST", "/backup/import");
        expect(sends).toHaveLength(1);
        expect(sends[0]?.url).toBe("/admin/v1/orgs/org_1/backup/import");
    });

    it("(b) 'Sobrescrever e importar' reenvia com ?sobrescrever=true e mostra o relatório do 200", async () => {
        let attempt = 0;
        const servidor = new FakeServer()
            .respond("POST", "/backup/import", () => {
                attempt += 1;
                return attempt === 1
                    ? apiError(409, "importacao_sobrescreve", "a importação atualizaria 4 linha(s)", DETAIL_OVERWRITES)
                    : { status: 200, corpo: REPORT };
            })
            .install();
        build();

        await choose(fileJson());
        await confirmImport();
        const alert = await screen.findByRole("alert");
        await userEvent.click(within(alert).getByRole("button", { name: "Sobrescrever e importar" }));

        expect(await screen.findByText(/linhas gravadas: 2 novas e 3 atualizadas/)).toBeInTheDocument();
        const sends = servidor.callsFor("POST", "/backup/import");
        expect(sends).toHaveLength(2);
        expect(sends[1]?.url).toBe("/admin/v1/orgs/org_1/backup/import?sobrescrever=true");
        expect(sends[1]?.raw).toBe(CONTENT_FILE);
        expect(screen.queryByText("Esta importação vai sobrescrever o que já existe")).toBeNull();
    });

    it("(c) conflito de nome (409 conflict) não oferece sobrescrever", async () => {
        new FakeServer()
            .respond("POST", "/backup/import", apiError(409, "conflict", "conflito", { conflicts: ["agente bia (slug)"], conflicts_more: 0 }))
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText("Importação recusada")).toBeInTheDocument();
        expect(within(alert).queryByRole("button", { name: "Sobrescrever e importar" })).toBeNull();
    });

    it("'Cancelar' tira a recusa da tela sem enviar nada de novo", async () => {
        const servidor = new FakeServer()
            .respond("POST", "/backup/import", apiError(409, "importacao_sobrescreve", "x", DETAIL_OVERWRITES))
            .install();
        build();

        await choose(fileJson());
        await confirmImport();
        const alert = await screen.findByRole("alert");
        await userEvent.click(within(alert).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        expect(servidor.callsFor("POST", "/backup/import")).toHaveLength(1);
    });

    it("a recusa sem detalhes ainda aparece com o aviso e o botão, sem tabela", async () => {
        new FakeServer()
            .respond("POST", "/backup/import", apiError(409, "importacao_sobrescreve", "x", null))
            .install();
        build();

        await choose(fileJson());
        await confirmImport();

        const alert = await screen.findByRole("alert");
        expect(within(alert).getByText(/0 linhas desta organização seriam atualizadas/)).toBeInTheDocument();
        expect(within(alert).queryByRole("table")).toBeNull();
        expect(within(alert).getByRole("button", { name: "Sobrescrever e importar" })).toBeInTheDocument();
    });
});
