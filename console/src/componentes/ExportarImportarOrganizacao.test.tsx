/**
 * Testes da seção "Exportar e importar a organização" (HR-48e-2), com o `fetch` falso.
 *
 * Cobrem: exportar sem e com conversas (query e nome do cabeçalho); importar com sucesso (texto do
 * arquivo no corpo, relatório, segredos a digitar de novo, índice pronto e índice pendente); conflito
 * `409` listado com "nada foi gravado"; arquivo inválido `400` com as versões; `413`; `403` com
 * "Tentar de novo" que recupera; arquivo acima de 64 MB recusado antes de enviar; confirmação com
 * `Esc` e com "Cancelar" sem enviar; `backup_incompleto` na exportação; nada disparado sozinho ao
 * montar a seção.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { ExportarImportarOrganizacao } from "./ExportarImportarOrganizacao";
import { LIMITE_IMPORTACAO_BYTES } from "../api/backup";
import { setLocale } from "../i18n";

const ORG = "org_1";
const NOME_DO_ARQUIVO = "org-origem-backup-20261121-1118.json";
const CONTEUDO_DO_ARQUIVO = JSON.stringify({ formato: "poph-backup-org", versao_formato: 1 });

const RELATORIO = {
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

const cabecalhoDeAnexo = { "content-disposition": `attachment; filename="${NOME_DO_ARQUIVO}"` };

let cliques: string[] = [];
let criarUrl: ReturnType<typeof vi.fn>;
let revogarUrl: ReturnType<typeof vi.fn>;
const urlOriginal = { criar: URL.createObjectURL, revogar: URL.revokeObjectURL };

beforeEach(() => {
    cliques = [];
    criarUrl = vi.fn(() => "blob:teste");
    revogarUrl = vi.fn();
    URL.createObjectURL = criarUrl as unknown as typeof URL.createObjectURL;
    URL.revokeObjectURL = revogarUrl as unknown as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
        cliques.push(this.download);
    });
});

afterEach(() => {
    URL.createObjectURL = urlOriginal.criar;
    URL.revokeObjectURL = urlOriginal.revogar;
});

function montar() {
    return render(<ExportarImportarOrganizacao org={ORG} />);
}

function arquivoJson(conteudo = CONTEUDO_DO_ARQUIVO, nome = "backup.json", tamanho?: number): File {
    const arquivo = new File([conteudo], nome, { type: "application/json" });
    if (tamanho !== undefined) {
        Object.defineProperty(arquivo, "size", { value: tamanho });
    }
    return arquivo;
}

function campoDeArquivo(): HTMLInputElement {
    const campo = document.querySelector('input[type="file"]');
    if (!(campo instanceof HTMLInputElement)) throw new Error("campo de arquivo ausente");
    return campo;
}

async function escolher(arquivo: File) {
    await userEvent.upload(campoDeArquivo(), arquivo);
}

/** Abre a confirmação de importação e confirma. */
async function confirmarImportacao() {
    await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
    const dialogo = await screen.findByRole("dialog");
    await userEvent.click(within(dialogo).getByRole("button", { name: "Importar agora" }));
}

describe("backup em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("mostra a seção, o relatório e os conflitos em inglês", async () => {
        setLocale("en-US");
        new ServidorFalso()
            .responder("GET", "/backup", {
                status: 200,
                texto: CONTEUDO_DO_ARQUIVO,
                cabecalhos: cabecalhoDeAnexo,
            })
            .responder("POST", "/backup/import", { status: 200, corpo: RELATORIO })
            .instalar();
        montar();

        expect(screen.getByRole("heading", { name: "Export and import the organization" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Download organization file" })).toBeInTheDocument();
        expect(screen.getByRole("checkbox", { name: /Include conversations/ })).toBeInTheDocument();

        await userEvent.click(screen.getByRole("button", { name: "Download organization file" }));
        expect(await screen.findByText(`File "${NOME_DO_ARQUIVO}" downloaded.`)).toBeInTheDocument();

        await escolher(arquivoJson());
        await userEvent.click(screen.getByRole("button", { name: /Import file into this organization/ }));
        const dialogo = await screen.findByRole("dialog");
        await userEvent.click(within(dialogo).getByRole("button", { name: "Import now" }));

        expect(await screen.findByText(/5 rows written: 2 new and 3 updated/)).toBeInTheDocument();
        expect(screen.getByText("Secrets to type again")).toBeInTheDocument();
        expect(screen.getByText(/1 integration arrived without the secret/)).toBeInTheDocument();
        expect(screen.getByText("Search ready")).toBeInTheDocument();
        const tabela = screen.getByRole("table", { name: "Rows written per table" });
        expect(within(tabela).getByText("Agents")).toBeInTheDocument();
        expect(within(tabela).getByText("Products")).toBeInTheDocument();
    });

    it("traduz a recusa de sobrescrita e os nomes das tabelas", async () => {
        setLocale("en-US");
        new ServidorFalso()
            .responder("POST", "/backup/import", {
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
            .instalar();
        montar();

        await escolher(arquivoJson());
        await userEvent.click(screen.getByRole("button", { name: /Import file into this organization/ }));
        const dialogo = await screen.findByRole("dialog");
        await userEvent.click(within(dialogo).getByRole("button", { name: "Import now" }));

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("This import will overwrite what already exists")).toBeInTheDocument();
        expect(within(alerta).getByText(/4 rows in this organization would be updated/)).toBeInTheDocument();
        const tabela = within(alerta).getByRole("table", { name: "Rows that would be overwritten per table" });
        expect(within(tabela).getByText("Agents")).toBeInTheDocument();
        const lista = within(alerta).getByRole("list", { name: "Examples of what would be overwritten" });
        const itens = within(lista).getAllByRole("listitem");
        expect(itens[0]).toHaveTextContent("Agents: bia");
        expect(itens[1]).toHaveTextContent("Products: dedetizacao");
        expect(within(alerta).getByRole("button", { name: "Overwrite and import" })).toBeEnabled();
    });
});

describe("exportar", () => {
    it("sem conversas: chama a rota sem query e baixa com o nome que o servidor mandou", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/backup", { status: 200, texto: CONTEUDO_DO_ARQUIVO, cabecalhos: cabecalhoDeAnexo })
            .instalar();
        montar();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        await waitFor(() => expect(cliques).toEqual([NOME_DO_ARQUIVO]));
        const chamadas = servidor.chamadasPara("GET", "/backup");
        expect(chamadas).toHaveLength(1);
        expect(chamadas[0]?.url).toBe("/admin/v1/orgs/org_1/backup");
        expect(criarUrl).toHaveBeenCalledTimes(1);
        expect(screen.getByText(`Arquivo "${NOME_DO_ARQUIVO}" baixado.`)).toBeInTheDocument();
    });

    it("com 'Incluir conversas' marcado: manda com_conversas=true", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/backup", { status: 200, texto: CONTEUDO_DO_ARQUIVO, cabecalhos: cabecalhoDeAnexo })
            .instalar();
        montar();

        const caixa = screen.getByRole("checkbox", { name: /Incluir conversas/ });
        expect(caixa).not.toBeChecked();
        await userEvent.click(caixa);
        expect(caixa).toBeChecked();
        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(servidor.chamadasPara("GET", "/backup")[0]?.url).toBe(
            "/admin/v1/orgs/org_1/backup?com_conversas=true",
        );
    });

    it("409 backup_incompleto explica o que fazer e não oferece tentar de novo", async () => {
        new ServidorFalso()
            .responder("GET", "/backup", erroApi(409, "backup_incompleto", "arquivo sem conteúdo", { name: "catalogo.png" }))
            .instalar();
        montar();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));

        expect(
            await screen.findByText(/Um arquivo da biblioteca desta organização não está no disco/),
        ).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tentar de novo" })).toBeNull();
        expect(cliques).toHaveLength(0);
    });

    it("erro de servidor no download mostra 'Tentar de novo' e o botão refaz a exportação", async () => {
        let tentativa = 0;
        const servidor = new ServidorFalso()
            .responder("GET", "/backup", () => {
                tentativa += 1;
                return tentativa === 1
                    ? erroApi(500, "interno", "falha")
                    : { status: 200, texto: CONTEUDO_DO_ARQUIVO, cabecalhos: cabecalhoDeAnexo };
            })
            .instalar();
        montar();

        await userEvent.click(screen.getByRole("button", { name: "Baixar arquivo da organização" }));
        const alerta = await screen.findByRole("alert");
        await userEvent.click(within(alerta).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() => expect(cliques).toEqual([NOME_DO_ARQUIVO]));
        expect(servidor.chamadasPara("GET", "/backup")).toHaveLength(2);
    });

    it("não dispara exportação nem importação ao montar a seção", () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/backup", { status: 200, texto: CONTEUDO_DO_ARQUIVO, cabecalhos: cabecalhoDeAnexo })
            .responder("POST", "/backup/import", { status: 200, corpo: RELATORIO })
            .instalar();
        montar();

        expect(servidor.chamadas).toHaveLength(0);
    });
});

describe("importar", () => {
    it("sucesso: envia o texto do arquivo como JSON cru e mostra o relatório com segredos e índice", async () => {
        const servidor = new ServidorFalso().responder("POST", "/backup/import", { status: 200, corpo: RELATORIO }).instalar();
        montar();

        await escolher(arquivoJson());
        expect(screen.getByText(/Escolhido: backup\.json/)).toBeInTheDocument();
        await confirmarImportacao();
        expect(await screen.findByText(/linhas gravadas: 2 novas e 3 atualizadas/)).toBeInTheDocument();

        const envio = servidor.chamadasPara("POST", "/backup/import")[0];
        expect(envio?.url).toBe("/admin/v1/orgs/org_1/backup/import");
        expect(envio?.cabecalhos["content-type"]).toBe("application/json");
        expect(envio?.bruto).toBe(CONTEUDO_DO_ARQUIVO);

        const tabela = screen.getByRole("table", { name: "Linhas gravadas por tabela" });
        expect(within(tabela).getByText("Agentes")).toBeInTheDocument();
        expect(within(tabela).getByText("Produtos")).toBeInTheDocument();
        expect(within(tabela).queryByText("Departamentos")).toBeNull();

        expect(screen.getByText("Segredos para digitar de novo")).toBeInTheDocument();
        expect(screen.getByText(/1 integração chegou sem o segredo/)).toBeInTheDocument();
        expect(screen.getByText(/aba Integrações do agente/)).toBeInTheDocument();
        expect(screen.getByText("Busca pronta")).toBeInTheDocument();
        expect(screen.queryByText("Importado, índice pendente")).toBeNull();
    });

    it("índice sem o modelo: aparece como importado com índice pendente, não como erro", async () => {
        new ServidorFalso()
            .responder("POST", "/backup/import", {
                status: 200,
                corpo: {
                    ...RELATORIO,
                    segredos_a_redigitar: 0,
                    indice: { base_conhecimento: { agentes: 0, trechos: 0 }, produtos: { trechos: 0 } },
                    aviso: "modelo de embedding indisponível (model_int8.onnx)",
                },
            })
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        expect(await screen.findByText("Importado, índice pendente")).toBeInTheDocument();
        expect(screen.getByText(/poph kb reindex/)).toBeInTheDocument();
        // O texto técnico do servidor não aparece na tela.
        expect(screen.queryByText(/model_int8/)).toBeNull();
        expect(screen.queryByText("Não consegui importar")).toBeNull();
        expect(screen.queryByText("Segredos para digitar de novo")).toBeNull();
    });

    it("409 conflito: lista até 5 conflitos, diz 'e mais N' e que nada foi gravado", async () => {
        new ServidorFalso()
            .responder(
                "POST",
                "/backup/import",
                erroApi(409, "conflict", "conflito", {
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
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("Importação recusada")).toBeInTheDocument();
        expect(within(alerta).getByText(/Nada foi gravado/)).toBeInTheDocument();
        const lista = within(alerta).getByRole("list", { name: "Conflitos encontrados" });
        const itens = within(lista).getAllByRole("listitem");
        expect(itens).toHaveLength(6);
        expect(itens[0]).toHaveTextContent("agente bia (slug)");
        expect(itens[5]).toHaveTextContent("e mais 3");
        // Conflito não se repete sozinho: não há "Tentar de novo" para ele.
        expect(within(alerta).queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("400 arquivo de outra versão: mostra as duas versões", async () => {
        new ServidorFalso()
            .responder(
                "POST",
                "/backup/import",
                erroApi(400, "invalid_backup", "versão de formato desconhecida: 99", { version: 99, supported_version: 1 }),
            )
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("Arquivo inválido")).toBeInTheDocument();
        expect(within(alerta).getByText(/é da versão 99; esta instalação entende a versão 1/)).toBeInTheDocument();
        expect(within(alerta).queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("400 arquivo corrompido: mensagem clara, sem o texto cru da API", async () => {
        new ServidorFalso()
            .responder("POST", "/backup/import", erroApi(400, "invalid_backup", "JSON inválido: EOF at line 1 column 1200"))
            .instalar();
        montar();

        await escolher(arquivoJson("{\"formato\":"));
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText(/não é um backup válido desta instalação/)).toBeInTheDocument();
        expect(alerta.textContent).not.toMatch(/EOF/);
    });

    it("413 pelo servidor: diz o limite de 64 MB e que nada foi gravado", async () => {
        new ServidorFalso().responder("POST", "/backup/import", erroApi(413, "payload_too_large", "grande")).instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("Arquivo grande demais")).toBeInTheDocument();
        expect(within(alerta).getByText(/passa de 64 MB\. Nada foi gravado/)).toBeInTheDocument();
    });

    it("403 com 'Tentar de novo': o botão pede a confirmação de novo e o segundo envio funciona", async () => {
        let tentativa = 0;
        const servidor = new ServidorFalso()
            .responder("POST", "/backup/import", () => {
                tentativa += 1;
                return tentativa === 1 ? erroApi(403, "forbidden", "sem permissão") : { status: 200, corpo: RELATORIO };
            })
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText(/não alcança esta organização/)).toBeInTheDocument();
        await userEvent.click(within(alerta).getByRole("button", { name: "Tentar de novo" }));

        const dialogo = await screen.findByRole("dialog");
        await userEvent.click(within(dialogo).getByRole("button", { name: "Importar agora" }));

        expect(await screen.findByText(/linhas gravadas/)).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/backup/import")).toHaveLength(2);
    });

    it("arquivo acima de 64 MB é recusado na tela e nada é enviado", async () => {
        const servidor = new ServidorFalso().responder("POST", "/backup/import", { status: 200, corpo: RELATORIO }).instalar();
        montar();

        await escolher(arquivoJson(CONTEUDO_DO_ARQUIVO, "grande.json", LIMITE_IMPORTACAO_BYTES + 1));

        expect(await screen.findByText(/acima do limite de 64 MB/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Importar arquivo nesta organização/ })).toBeDisabled();
        expect(servidor.chamadasPara("POST", "/backup/import")).toHaveLength(0);
    });

    it("sem arquivo escolhido, o botão de importar fica desligado", () => {
        montar();

        expect(screen.getByRole("button", { name: /Importar arquivo nesta organização/ })).toBeDisabled();
    });

    it("Esc e 'Cancelar' fecham a confirmação sem enviar nada", async () => {
        const servidor = new ServidorFalso().responder("POST", "/backup/import", { status: 200, corpo: RELATORIO }).instalar();
        montar();

        await escolher(arquivoJson());
        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        await screen.findByRole("dialog");
        await userEvent.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        const dialogo = await screen.findByRole("dialog");
        await userEvent.click(within(dialogo).getByRole("button", { name: "Cancelar" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        expect(servidor.chamadasPara("POST", "/backup/import")).toHaveLength(0);
    });

    it("a confirmação explica que importar não apaga o que já existe", async () => {
        montar();

        await escolher(arquivoJson());
        await userEvent.click(screen.getByRole("button", { name: /Importar arquivo nesta organização/ }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText(/não apaga o que já existe aqui/)).toBeInTheDocument();
    });
});

/** Detalhe da recusa de sobrescrita, no formato que o servidor devolve (HR-48e-3). */
const DETALHE_SOBRESCREVE = {
    tabelas: { agents: 1, products: 3 },
    linhas: 4,
    exemplos: ["agents: bia", "products: dedetizacao", "product_categories: Pragas"],
};

describe("recusa de sobrescrita (HR-48e-4)", () => {
    it("(a) recusa mostra o que seria sobrescrito, o total, os exemplos e o botão de autorizar, sem enviar de novo", async () => {
        const servidor = new ServidorFalso()
            .responder(
                "POST",
                "/backup/import",
                erroApi(409, "importacao_sobrescreve", "a importação atualizaria 4 linha(s)", DETALHE_SOBRESCREVE),
            )
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("Esta importação vai sobrescrever o que já existe")).toBeInTheDocument();
        expect(within(alerta).getByText(/4 linhas desta organização seriam atualizadas/)).toBeInTheDocument();
        expect(within(alerta).getByText(/Nada foi gravado/)).toBeInTheDocument();

        const tabela = within(alerta).getByRole("table", { name: "Linhas que seriam sobrescritas por tabela" });
        expect(within(tabela).getByText("Agentes")).toBeInTheDocument();
        expect(within(tabela).getByText("Produtos")).toBeInTheDocument();

        const lista = within(alerta).getByRole("list", { name: "Exemplos do que seria sobrescrito" });
        const itens = within(lista).getAllByRole("listitem");
        expect(itens[0]).toHaveTextContent("Agentes: bia");
        expect(itens[1]).toHaveTextContent("Produtos: dedetizacao");
        expect(itens[2]).toHaveTextContent("Categorias de produto: Pragas");

        expect(within(alerta).getByRole("button", { name: "Sobrescrever e importar" })).toBeEnabled();
        // Só a tentativa sem autorização foi enviada: a recusa não reenvia sozinha.
        const envios = servidor.chamadasPara("POST", "/backup/import");
        expect(envios).toHaveLength(1);
        expect(envios[0]?.url).toBe("/admin/v1/orgs/org_1/backup/import");
    });

    it("(b) 'Sobrescrever e importar' reenvia com ?sobrescrever=true e mostra o relatório do 200", async () => {
        let tentativa = 0;
        const servidor = new ServidorFalso()
            .responder("POST", "/backup/import", () => {
                tentativa += 1;
                return tentativa === 1
                    ? erroApi(409, "importacao_sobrescreve", "a importação atualizaria 4 linha(s)", DETALHE_SOBRESCREVE)
                    : { status: 200, corpo: RELATORIO };
            })
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();
        const alerta = await screen.findByRole("alert");
        await userEvent.click(within(alerta).getByRole("button", { name: "Sobrescrever e importar" }));

        expect(await screen.findByText(/linhas gravadas: 2 novas e 3 atualizadas/)).toBeInTheDocument();
        const envios = servidor.chamadasPara("POST", "/backup/import");
        expect(envios).toHaveLength(2);
        expect(envios[1]?.url).toBe("/admin/v1/orgs/org_1/backup/import?sobrescrever=true");
        expect(envios[1]?.bruto).toBe(CONTEUDO_DO_ARQUIVO);
        // A recusa some quando a importação passa.
        expect(screen.queryByText("Esta importação vai sobrescrever o que já existe")).toBeNull();
    });

    it("(c) conflito de nome (409 conflict) não oferece sobrescrever", async () => {
        new ServidorFalso()
            .responder("POST", "/backup/import", erroApi(409, "conflict", "conflito", { conflicts: ["agente bia (slug)"], conflicts_more: 0 }))
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText("Importação recusada")).toBeInTheDocument();
        expect(within(alerta).queryByRole("button", { name: "Sobrescrever e importar" })).toBeNull();
    });

    it("'Cancelar' tira a recusa da tela sem enviar nada de novo", async () => {
        const servidor = new ServidorFalso()
            .responder("POST", "/backup/import", erroApi(409, "importacao_sobrescreve", "x", DETALHE_SOBRESCREVE))
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();
        const alerta = await screen.findByRole("alert");
        await userEvent.click(within(alerta).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        expect(servidor.chamadasPara("POST", "/backup/import")).toHaveLength(1);
    });

    it("a recusa sem detalhes ainda aparece com o aviso e o botão, sem tabela", async () => {
        new ServidorFalso()
            .responder("POST", "/backup/import", erroApi(409, "importacao_sobrescreve", "x", null))
            .instalar();
        montar();

        await escolher(arquivoJson());
        await confirmarImportacao();

        const alerta = await screen.findByRole("alert");
        expect(within(alerta).getByText(/0 linhas desta organização seriam atualizadas/)).toBeInTheDocument();
        expect(within(alerta).queryByRole("table")).toBeNull();
        expect(within(alerta).getByRole("button", { name: "Sobrescrever e importar" })).toBeInTheDocument();
    });
});
