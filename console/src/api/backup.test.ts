
import { afterEach, describe, expect, it } from "vitest";
import { ApiError } from "./client";
import { setLocale } from "../i18n";
import {
    SHOWN_CONFLICTS,
    LIMIT_IMPORT_BYTES,
    exportPath,
    importPath,
    backupFailure,
    tableName,
    exceedsLimit,
    overwriteRefusal,
    conflictText,
    sampleText,
} from "./backup";

describe("caminhos", () => {
    it("exportação sem conversas não tem query; com conversas, com_conversas=true", () => {
        expect(exportPath("org_1", false)).toBe("/admin/v1/orgs/org_1/backup");
        expect(exportPath("org_1", true)).toBe("/admin/v1/orgs/org_1/backup?com_conversas=true");
    });

    it("importação vai para a org do caminho", () => {
        expect(importPath("org_9")).toBe("/admin/v1/orgs/org_9/backup/import");
    });

    it("sem autorização não há query; com autorização, sobrescrever=true", () => {
        expect(importPath("org_9", false)).toBe("/admin/v1/orgs/org_9/backup/import");
        expect(importPath("org_9", true)).toBe("/admin/v1/orgs/org_9/backup/import?sobrescrever=true");
    });

    it("o id da org é codificado no caminho", () => {
        expect(exportPath("a/b", false)).toBe("/admin/v1/orgs/a%2Fb/backup");
    });
});

describe("limite e nomes", () => {
    it("o teto é 64 MB: exatamente no limite passa, um byte a mais não", () => {
        expect(LIMIT_IMPORT_BYTES).toBe(64 * 1024 * 1024);
        expect(exceedsLimit(LIMIT_IMPORT_BYTES)).toBe(false);
        expect(exceedsLimit(LIMIT_IMPORT_BYTES + 1)).toBe(true);
    });

    it("tabela conhecida ganha nome de operador; desconhecida mantém o técnico", () => {
        expect(tableName("agents")).toBe("Agentes");
        expect(tableName("cart_items")).toBe("Itens de orçamento");
        expect(tableName("tabela_nova")).toBe("tabela_nova");
    });
});

describe("textoDeConflito", () => {
    it("tira a crase e troca o nome técnico da tabela pelo de operador", () => {
        expect(conflictText("`product_categories` 01a1-x (o id do arquivo já existe)")).toBe(
            "Categorias de produto: 01a1-x (o id do arquivo já existe)",
        );
    });

    it("texto sem tabela conhecida fica como veio, sem crase", () => {
        expect(conflictText("agente bia (slug)")).toBe("agente bia (slug)");
        expect(conflictText("`tabela_nova` x")).toBe("tabela_nova x");
    });
});

describe("falhaDeBackup", () => {
    it("409 conflito lista até 5 e soma o resto ao 'e mais'", () => {
        const items = ["a", "b", "c", "d", "e", "f", "g"];
        const failure = backupFailure(
            new ApiError(409, "conflict", "cru", { conflicts: items.slice(0, 5), conflicts_more: 2 }),
            "importar",
        );
        expect(failure["title"]).toBe("Importação recusada");
        expect(failure.texto).toMatch(/Nada foi gravado/);
        expect(failure["conflicts"]?.["items"]).toHaveLength(SHOWN_CONFLICTS);
        expect(failure["conflicts"]?.["more"]).toBe(2);
        expect(failure["canRetry"]).toBe(false);
    });

    it("400 invalid_backup com versões: leva as duas versões", () => {
        const failure = backupFailure(
            new ApiError(400, "invalid_backup", "cru", { version: 7, supported_version: 1 }),
            "importar",
        );
        expect(failure["versions"]).toEqual({ "file": 7, "installation": 1 });
        expect(failure.texto).not.toMatch(/cru/);
    });

    it("400 invalid_backup sem versões não inventa números", () => {
        const failure = backupFailure(new ApiError(400, "invalid_backup", "cru", null), "importar");
        expect(failure["versions"]).toBeUndefined();
    });

    it("413 diz o limite de 64 MB e não oferece nova tentativa", () => {
        const failure = backupFailure(new ApiError(413, "payload_too_large", "cru"), "importar");
        expect(failure.texto).toMatch(/64 MB/);
        expect(failure["canRetry"]).toBe(false);
    });

    it("401 pede para entrar de novo, sem tentar de novo", () => {
        const failure = backupFailure(new ApiError(401, "invalid_token", "cru"), "exportar");
        expect(failure["title"]).toBe("Sessão encerrada");
        expect(failure["canRetry"]).toBe(false);
    });

    it("403 e 500 podem tentar de novo", () => {
        expect(backupFailure(new ApiError(403, "forbidden", "x"), "exportar")["canRetry"]).toBe(true);
        expect(backupFailure(new ApiError(500, "interno", "x"), "importar")["canRetry"]).toBe(true);
    });

    it("404 diz que a organização não existe", () => {
        const failure = backupFailure(new ApiError(404, "not_found", "x"), "exportar");
        expect(failure["title"]).toBe("Organização não encontrada");
    });

    it("backup_incompleto explica a biblioteca sem conteúdo e não oferece nova tentativa", () => {
        const failure = backupFailure(new ApiError(409, "backup_incompleto", "x", { name: "a.png" }), "exportar");
        expect(failure.texto).toMatch(/biblioteca/);
        expect(failure.texto).not.toMatch(/a\.png/);
        expect(failure["canRetry"]).toBe(false);
    });

    it("erro que não é da API vira mensagem genérica", () => {
        const failure = backupFailure(new Error("bagunçado"), "importar");
        expect(failure.texto).toBe("Algo deu errado. Tente de novo.");
        expect(failure["canRetry"]).toBe(true);
    });

    it("sem servidor pede para conferir se a instância está no ar", () => {
        const failure = backupFailure(new ApiError(0, "rede", "x"), "exportar");
        expect(failure.texto).toMatch(/servidor está no ar/);
    });
});

describe("backup em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("traduz nomes de tabela, conflitos e erros para inglês", () => {
        setLocale("en-US");

        expect(tableName("agents")).toBe("Agents");
        expect(tableName("cart_items")).toBe("Cart items");
        expect(tableName("tabela_nova")).toBe("tabela_nova");
        expect(conflictText("`product_categories` 01a1-x")).toBe("Product categories: 01a1-x");
        expect(sampleText("agents: bia")).toBe("Agents: bia");

        const session = backupFailure(new ApiError(401, "invalid_token", "cru"), "exportar");
        expect(session["title"]).toBe("Session ended");
        expect(session.texto).toMatch(/Sign in again/);

        const conflict = backupFailure(
            new ApiError(409, "conflict", "cru", { conflicts: ["a"], conflicts_more: 1 }),
            "importar",
        );
        expect(conflict["title"]).toBe("Import refused");
        expect(conflict.texto).toMatch(/Nothing was written/);

        const large = backupFailure(new ApiError(413, "payload_too_large", "cru"), "importar");
        expect(large["title"]).toBe("File too large");
    });
});

describe("recusa de sobrescrita (HR-48e-4)", () => {
    it("lê tabelas, total e exemplos do detalhe, pelo code importacao_sobrescreve", () => {
        const refusal = overwriteRefusal(
            new ApiError(409, "importacao_sobrescreve", "x", {
                tabelas: { agents: 1, products: 3 },
                "linhas": 4,
                "exemplos": ["agents: bia", "products: dedetizacao"],
            }),
        );
        expect(refusal).toEqual({
            tabelas: { agents: 1, products: 3 },
            "linhas": 4,
            "exemplos": ["agents: bia", "products: dedetizacao"],
        });
    });

    it("limita os exemplos a 5 e descarta tabelas com valor inválido", () => {
        const refusal = overwriteRefusal(
            new ApiError(409, "importacao_sobrescreve", "x", {
                tabelas: { agents: 2, products: "três", zero: 0 },
                "exemplos": ["a: 1", "a: 2", "a: 3", "a: 4", "a: 5", "a: 6"],
            }),
        );
        expect(refusal?.tabelas).toEqual({ agents: 2 });
        expect(refusal?.["linhas"]).toBe(2);
        expect(refusal?.["exemplos"]).toHaveLength(5);
    });

    it("não é recusa de sobrescrita: conflito de nome, outro status ou erro que não é da API", () => {
        expect(overwriteRefusal(new ApiError(409, "conflict", "x", {}))).toBeNull();
        expect(overwriteRefusal(new ApiError(400, "importacao_sobrescreve", "x", {}))).toBeNull();
        expect(overwriteRefusal(new Error("solto"))).toBeNull();
    });

    it("exemplo vira nome de operador da tabela; sem tabela conhecida, fica como veio", () => {
        expect(sampleText("agents: bia")).toBe("Agentes: bia");
        expect(sampleText("product_categories: Pragas")).toBe("Categorias de produto: Pragas");
        expect(sampleText("`tabela_nova`: x")).toBe("tabela_nova: x");
        expect(sampleText("sem dois pontos")).toBe("sem dois pontos");
    });

    it("a falha genérica da recusa não oferece tentar de novo", () => {
        const failure = backupFailure(new ApiError(409, "importacao_sobrescreve", "x", null), "importar");
        expect(failure["title"]).toBe("Esta importação vai sobrescrever o que já existe");
        expect(failure["canRetry"]).toBe(false);
    });
});
