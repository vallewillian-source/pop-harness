/**
 * Funções puras de exportar e importar a organização (HR-48e-2): caminhos, limite, nomes de tabela e
 * a tradução de cada erro da API em texto de operador. Quem decide é o `code`, nunca a mensagem.
 */

import { describe, expect, it } from "vitest";
import { ErroApi } from "./cliente";
import {
    CONFLITOS_MOSTRADOS,
    LIMITE_IMPORTACAO_BYTES,
    caminhoDaExportacao,
    caminhoDaImportacao,
    falhaDeBackup,
    nomeDaTabela,
    passaDoLimite,
    recusaDeSobrescrita,
    textoDeConflito,
    textoDeExemplo,
} from "./backup";

describe("caminhos", () => {
    it("exportação sem conversas não tem query; com conversas, com_conversas=true", () => {
        expect(caminhoDaExportacao("org_1", false)).toBe("/admin/v1/orgs/org_1/backup");
        expect(caminhoDaExportacao("org_1", true)).toBe("/admin/v1/orgs/org_1/backup?com_conversas=true");
    });

    it("importação vai para a org do caminho", () => {
        expect(caminhoDaImportacao("org_9")).toBe("/admin/v1/orgs/org_9/backup/import");
    });

    it("sem autorização não há query; com autorização, sobrescrever=true", () => {
        expect(caminhoDaImportacao("org_9", false)).toBe("/admin/v1/orgs/org_9/backup/import");
        expect(caminhoDaImportacao("org_9", true)).toBe("/admin/v1/orgs/org_9/backup/import?sobrescrever=true");
    });

    it("o id da org é codificado no caminho", () => {
        expect(caminhoDaExportacao("a/b", false)).toBe("/admin/v1/orgs/a%2Fb/backup");
    });
});

describe("limite e nomes", () => {
    it("o teto é 64 MB: exatamente no limite passa, um byte a mais não", () => {
        expect(LIMITE_IMPORTACAO_BYTES).toBe(64 * 1024 * 1024);
        expect(passaDoLimite(LIMITE_IMPORTACAO_BYTES)).toBe(false);
        expect(passaDoLimite(LIMITE_IMPORTACAO_BYTES + 1)).toBe(true);
    });

    it("tabela conhecida ganha nome de operador; desconhecida mantém o técnico", () => {
        expect(nomeDaTabela("agents")).toBe("Agentes");
        expect(nomeDaTabela("cart_items")).toBe("Itens de orçamento");
        expect(nomeDaTabela("tabela_nova")).toBe("tabela_nova");
    });
});

describe("textoDeConflito", () => {
    it("tira a crase e troca o nome técnico da tabela pelo de operador", () => {
        expect(textoDeConflito("`product_categories` 01a1-x (o id do arquivo já existe)")).toBe(
            "Categorias de produto: 01a1-x (o id do arquivo já existe)",
        );
    });

    it("texto sem tabela conhecida fica como veio, sem crase", () => {
        expect(textoDeConflito("agente bia (slug)")).toBe("agente bia (slug)");
        expect(textoDeConflito("`tabela_nova` x")).toBe("tabela_nova x");
    });
});

describe("falhaDeBackup", () => {
    it("409 conflito lista até 5 e soma o resto ao 'e mais'", () => {
        const itens = ["a", "b", "c", "d", "e", "f", "g"];
        const falha = falhaDeBackup(
            new ErroApi(409, "conflict", "cru", { conflicts: itens.slice(0, 5), conflicts_more: 2 }),
            "importar",
        );
        expect(falha.titulo).toBe("Importação recusada");
        expect(falha.texto).toMatch(/Nada foi gravado/);
        expect(falha.conflitos?.itens).toHaveLength(CONFLITOS_MOSTRADOS);
        expect(falha.conflitos?.mais).toBe(2);
        expect(falha.podeTentarDeNovo).toBe(false);
    });

    it("400 invalid_backup com versões: leva as duas versões", () => {
        const falha = falhaDeBackup(
            new ErroApi(400, "invalid_backup", "cru", { version: 7, supported_version: 1 }),
            "importar",
        );
        expect(falha.versoes).toEqual({ arquivo: 7, instalacao: 1 });
        expect(falha.texto).not.toMatch(/cru/);
    });

    it("400 invalid_backup sem versões não inventa números", () => {
        const falha = falhaDeBackup(new ErroApi(400, "invalid_backup", "cru", null), "importar");
        expect(falha.versoes).toBeUndefined();
    });

    it("413 diz o limite de 64 MB e não oferece nova tentativa", () => {
        const falha = falhaDeBackup(new ErroApi(413, "payload_too_large", "cru"), "importar");
        expect(falha.texto).toMatch(/64 MB/);
        expect(falha.podeTentarDeNovo).toBe(false);
    });

    it("401 pede para entrar de novo, sem tentar de novo", () => {
        const falha = falhaDeBackup(new ErroApi(401, "invalid_token", "cru"), "exportar");
        expect(falha.titulo).toBe("Sessão encerrada");
        expect(falha.podeTentarDeNovo).toBe(false);
    });

    it("403 e 500 podem tentar de novo", () => {
        expect(falhaDeBackup(new ErroApi(403, "forbidden", "x"), "exportar").podeTentarDeNovo).toBe(true);
        expect(falhaDeBackup(new ErroApi(500, "interno", "x"), "importar").podeTentarDeNovo).toBe(true);
    });

    it("404 diz que a organização não existe", () => {
        const falha = falhaDeBackup(new ErroApi(404, "not_found", "x"), "exportar");
        expect(falha.titulo).toBe("Organização não encontrada");
    });

    it("backup_incompleto explica a biblioteca sem conteúdo e não oferece nova tentativa", () => {
        const falha = falhaDeBackup(new ErroApi(409, "backup_incompleto", "x", { name: "a.png" }), "exportar");
        expect(falha.texto).toMatch(/biblioteca/);
        expect(falha.texto).not.toMatch(/a\.png/);
        expect(falha.podeTentarDeNovo).toBe(false);
    });

    it("erro que não é da API vira mensagem genérica", () => {
        const falha = falhaDeBackup(new Error("bagunçado"), "importar");
        expect(falha.texto).toBe("Algo deu errado. Tente de novo.");
        expect(falha.podeTentarDeNovo).toBe(true);
    });

    it("sem servidor pede para conferir se a instância está no ar", () => {
        const falha = falhaDeBackup(new ErroApi(0, "rede", "x"), "exportar");
        expect(falha.texto).toMatch(/servidor está no ar/);
    });
});

describe("recusa de sobrescrita (HR-48e-4)", () => {
    it("lê tabelas, total e exemplos do detalhe, pelo code importacao_sobrescreve", () => {
        const recusa = recusaDeSobrescrita(
            new ErroApi(409, "importacao_sobrescreve", "x", {
                tabelas: { agents: 1, products: 3 },
                linhas: 4,
                exemplos: ["agents: bia", "products: dedetizacao"],
            }),
        );
        expect(recusa).toEqual({
            tabelas: { agents: 1, products: 3 },
            linhas: 4,
            exemplos: ["agents: bia", "products: dedetizacao"],
        });
    });

    it("limita os exemplos a 5 e descarta tabelas com valor inválido", () => {
        const recusa = recusaDeSobrescrita(
            new ErroApi(409, "importacao_sobrescreve", "x", {
                tabelas: { agents: 2, products: "três", zero: 0 },
                exemplos: ["a: 1", "a: 2", "a: 3", "a: 4", "a: 5", "a: 6"],
            }),
        );
        expect(recusa?.tabelas).toEqual({ agents: 2 });
        expect(recusa?.linhas).toBe(2);
        expect(recusa?.exemplos).toHaveLength(5);
    });

    it("não é recusa de sobrescrita: conflito de nome, outro status ou erro que não é da API", () => {
        expect(recusaDeSobrescrita(new ErroApi(409, "conflict", "x", {}))).toBeNull();
        expect(recusaDeSobrescrita(new ErroApi(400, "importacao_sobrescreve", "x", {}))).toBeNull();
        expect(recusaDeSobrescrita(new Error("solto"))).toBeNull();
    });

    it("exemplo vira nome de operador da tabela; sem tabela conhecida, fica como veio", () => {
        expect(textoDeExemplo("agents: bia")).toBe("Agentes: bia");
        expect(textoDeExemplo("product_categories: Pragas")).toBe("Categorias de produto: Pragas");
        expect(textoDeExemplo("`tabela_nova`: x")).toBe("tabela_nova: x");
        expect(textoDeExemplo("sem dois pontos")).toBe("sem dois pontos");
    });

    it("a falha genérica da recusa não oferece tentar de novo", () => {
        const falha = falhaDeBackup(new ErroApi(409, "importacao_sobrescreve", "x", null), "importar");
        expect(falha.titulo).toBe("Esta importação vai sobrescrever o que já existe");
        expect(falha.podeTentarDeNovo).toBe(false);
    });
});
