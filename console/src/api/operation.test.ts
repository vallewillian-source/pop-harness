
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "./client";
import {
    fetchMetrics,
    fetchHealth,
    parseMetrics,
    parseReport,
    listPlugins,
    reloadCluster,
} from "./operation";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";

afterEach(() => {
    vi.unstubAllGlobals();
});

const HEALTH_OK = {
    status: "ok",
    version: "0.9.1",
    uptime_seconds: 3725,
    databases: { harness: 12, rag: 3 },
};

describe("interpretarMetricas", () => {
    it("lê só os nomes que a tela explica e ignora comentários, histogramas e linhas quebradas", () => {
        const texto = [
            "# HELP poph_fila_turnos Turnos esperando na fila.",
            "# TYPE poph_fila_turnos gauge",
            "poph_fila_turnos 4",
            "poph_fila_saida 0",
            'poph_latencia_etapa_segundos_bucket{etapa="llm",le="0.5"} 9',
            "poph_latencia_etapa_segundos_sum 1.25",
            "poph_tokens_total 1500",
            "poph_chamadas_llm_total NaN",
            "poph_nome_que_nao_existe 7",
            "linha sem valor",
            "",
        ].join("\n");

        expect(parseMetrics(texto)).toEqual({
            "filaTurnos": 4,
            "filaSaida": 0,
            tokens: 1500,
        });
    });

    it("devolve vazio quando o texto não tem nenhum nome conhecido (a tela mostra 'não informado')", () => {
        expect(parseMetrics("# nada aqui\n")).toEqual({});
    });
});

describe("consultarSaude", () => {
    it("200 vira situação ok com o corpo", async () => {
        const servidor = new FakeServer()["responder"]("GET", "/healthz", { status: 200, "corpo": HEALTH_OK })["instalar"]();

        const response = await fetchHealth();

        expect(response.situacao).toBe("ok");
        expect(response["saude"].version).toBe("0.9.1");
        expect(servidor["chamadasPara"]("GET", "/healthz")).toHaveLength(1);
    });

    it("503 com corpo vira situação degradada, sem jogar fora o resumo", async () => {
        new FakeServer()["responder"]("GET", "/healthz", {
                status: 503,
                "corpo": { ...HEALTH_OK, status: "degraded" },
            })["instalar"]();

        const response = await fetchHealth();

        expect(response.situacao).toBe("degradada");
        expect(response["saude"].databases.harness).toBe(12);
    });

    it("500 sem resumo vira erro com o status", async () => {
        new FakeServer()["responder"]("GET", "/healthz", apiError(500, "interno", "falha"))["instalar"]();

        await expect(fetchHealth()).rejects.toMatchObject({ status: 500 });
    });

    it("sem conexão com o servidor vira erro de rede (status 0)", async () => {
        vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))));

        const failure = await fetchHealth().catch((error: unknown) => error);

        expect(failure).toBeInstanceOf(ApiError);
        expect((failure as ApiError)["semServidor"]).toBe(true);
    });
});

describe("consultarMetricas", () => {
    it("lê o texto do /metrics sem tentar interpretar como JSON", async () => {
        new FakeServer()["responder"]("GET", "/metrics", {
                status: 200,
                texto: "poph_fila_turnos 2\npoph_agentes_carregados 1\n",
            })["instalar"]();

        expect(await fetchMetrics()).toEqual({ "filaTurnos": 2, "agentesCarregados": 1 });
    });

    it("status diferente de 200 vira erro", async () => {
        new FakeServer()["responder"]("GET", "/metrics", { status: 500, "corpo": { code: "x", message: "y" } })["instalar"]();

        await expect(fetchMetrics()).rejects.toMatchObject({ status: 500 });
    });
});

describe("listarPlugins (HR-48c)", () => {
    it("chama a lista da org com o token da sessão e mantém a ordem recebida", async () => {
        const servidor = new FakeServer()["responder"]("GET", "/admin/v1/orgs/org_1/manifests", {
                status: 200,
                "corpo": {
                    items: [
                        { kind: "eco", situacao: "ok", nome: "Eco", versao: "0.1.0" },
                        { kind: "whatsapp", situacao: "indisponivel", mensagem: "sem resposta" },
                    ],
                },
            })["instalar"]();

        const items = await listPlugins("org_1");

        expect(items).toEqual([
            { kind: "eco", situacao: "ok", nome: "Eco", versao: "0.1.0" },
            { kind: "whatsapp", situacao: "indisponivel", mensagem: "sem resposta" },
        ]);
        expect(servidor["calls"][0]?.url).toBe("/admin/v1/orgs/org_1/manifests");
    });

    it("plugin indisponível sem mensagem ganha a frase de operação, sem nome nem versão", async () => {
        new FakeServer()["responder"]("GET", "/manifests", {
                status: 200,
                "corpo": { items: [{ kind: "quebrado", situacao: "indisponivel" }] },
            })["instalar"]();

        const [item] = await listPlugins("org_1");

        expect(item?.situacao).toBe("indisponivel");
        expect(item?.mensagem).toMatch(/não respondeu/);
        expect(item?.nome).toBeUndefined();
        expect(item?.versao).toBeUndefined();
    });

    it("item sem tipo de texto é descartado, e situação que não é ok vira indisponível", async () => {
        new FakeServer()["responder"]("GET", "/manifests", {
                status: 200,
                "corpo": {
                    items: [
                        { situacao: "ok", nome: "Sem tipo" },
                        null,
                        { kind: 7, situacao: "ok" },
                        { kind: "estranho", situacao: "talvez", mensagem: "algo" },
                    ],
                },
            })["instalar"]();

        const items = await listPlugins("org_1");

        expect(items).toEqual([{ kind: "estranho", situacao: "indisponivel", mensagem: "algo" }]);
    });

    it("resposta sem items vira lista vazia, não erro", async () => {
        new FakeServer()["responder"]("GET", "/manifests", { status: 200, "corpo": {} })["instalar"]();

        expect(await listPlugins("org_1")).toEqual([]);
    });

    it("403 de outra org chega como erro da API, para a tela explicar", async () => {
        new FakeServer()["responder"]("GET", "/manifests", apiError(403, "forbidden", "outra org"))["instalar"]();

        await expect(listPlugins("org_2")).rejects.toMatchObject({ status: 403 });
    });
});

describe("recarregarCluster (HR-48f)", () => {
    const CLUSTER_MAP = {
        "modo": "cluster",
        instance_id: "a",
        default_instance: "a",
        "segredo_configurado": true,
        "instancias": [
            { id: "a", url: "http://a.internal:8080", local: true },
            { id: "b", url: "http://b.internal:8080", local: false },
        ],
        orgs: { org_1: "a", org_2: "b" },
    };
    const SINGLE_MAP = {
        "modo": "instancia_unica",
        instance_id: "",
        default_instance: null,
        "segredo_configurado": false,
        "instancias": [],
        orgs: {},
    };

    it("faz POST na rota global, sem corpo, e devolve o relatório tipado", async () => {
        const servidor = new FakeServer()["responder"]("POST", "/admin/v1/cluster/reload", {
                status: 200,
                "corpo": {
                    "mudou": true,
                    "fingerprint_antes": "aaaa1111bbbb2222",
                    "fingerprint_depois": "cccc3333dddd4444",
                    "antes": SINGLE_MAP,
                    "depois": CLUSTER_MAP,
                    "orgs_que_trocaram": [{ org: "org_2", de: null, para: "b" }],
                    "instancias_adicionadas": ["b"],
                    "instancias_removidas": [],
                },
            })["instalar"]();

        const report = await reloadCluster();

        const call = servidor["chamadasPara"]("POST", "/admin/v1/cluster/reload")[0];
        expect(call?.["metodo"]).toBe("POST");
        expect(call?.["bruto"]).toBeUndefined();
        expect(report["mudou"]).toBe(true);
        expect(report["impressaoAntes"]).toBe("aaaa1111bbbb2222");
        expect(report["impressaoDepois"]).toBe("cccc3333dddd4444");
        expect(report["antes"]["modo"]).toBe("instancia_unica");
        expect(report["depois"]).toMatchObject({
            "modo": "cluster",
            "instanciaLocal": "a",
            "instanciaPadrao": "a",
            "segredoConfigurado": true,
            "quantidadeDeOrgs": 2,
            "instancias": [
                { id: "a", local: true },
                { id: "b", local: false },
            ],
        });
        expect(report["orgsQueTrocaram"]).toEqual([{ org: "org_2", de: null, para: "b" }]);
        expect(report["instanciasAdicionadas"]).toEqual(["b"]);
        expect(report["instanciasRemovidas"]).toEqual([]);
    });

    it("não leva o endereço das instâncias para o resultado", async () => {
        new FakeServer()["responder"]("POST", "/admin/v1/cluster/reload", {
                status: 200,
                "corpo": { "mudou": false, "fingerprint_antes": "x", "fingerprint_depois": "x", "antes": CLUSTER_MAP, "depois": CLUSTER_MAP, "orgs_que_trocaram": [], "instancias_adicionadas": [], "instancias_removidas": [] },
            })["instalar"]();

        const report = await reloadCluster();

        expect(JSON.stringify(report)).not.toContain("internal:8080");
    });

    it("400 cluster_invalido sai como ErroApi com a lista de problemas em detalhes", async () => {
        new FakeServer()["responder"](
                "POST",
                "/admin/v1/cluster/reload",
                apiError(400, "cluster_invalido", "cluster.toml inválido: instância sem endereço", {
                    "arquivo": "/etc/poph/cluster.toml",
                    "problemas": ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
                }),
            )["instalar"]();

        const error = await reloadCluster().catch((e: unknown) => e);

        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError).status).toBe(400);
        expect((error as ApiError)["codigo"]).toBe("cluster_invalido");
        expect((error as ApiError)["detalhes"]).toMatchObject({
            "problemas": ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
        });
    });

    it("403 sai como ErroApi com status 403 (token de org)", async () => {
        new FakeServer()["responder"]("POST", "/admin/v1/cluster/reload", apiError(403, "forbidden", "token de org"))["instalar"]();

        const error = await reloadCluster().catch((e: unknown) => e);

        expect((error as ApiError).status).toBe(403);
    });

    it("corpo de sucesso sem o resumo esperado vira erro de formato, não resultado inventado", async () => {
        new FakeServer()["responder"]("POST", "/admin/v1/cluster/reload", { status: 200, "corpo": { ok: true } })["instalar"]();

        const error = await reloadCluster().catch((e: unknown) => e);

        expect(error).toBeInstanceOf(ApiError);
        expect((error as ApiError)["codigo"]).toBe("formato");
    });
});

describe("interpretarRelatorio (HR-48f)", () => {
    it("modo instância única sem id local e sem padrão", () => {
        const report = parseReport({
            "mudou": false,
            "fingerprint_antes": "f",
            "fingerprint_depois": "f",
            "antes": { "modo": "instancia_unica", instance_id: "", default_instance: null, "segredo_configurado": false, "instancias": [], orgs: {} },
            "depois": { "modo": "instancia_unica", instance_id: "", default_instance: null, "segredo_configurado": false, "instancias": [], orgs: {} },
            "orgs_que_trocaram": [],
            "instancias_adicionadas": [],
            "instancias_removidas": [],
        });

        expect(report["depois"]).toEqual({
            "modo": "instancia_unica",
            "instanciaLocal": "",
            "instanciaPadrao": null,
            "segredoConfigurado": false,
            "instancias": [],
            "quantidadeDeOrgs": 0,
        });
        expect(report["orgsQueTrocaram"]).toEqual([]);
    });

    it("itens malformados em listas são descartados sem derrubar a leitura", () => {
        const report = parseReport({
            "mudou": true,
            "fingerprint_antes": "a",
            "fingerprint_depois": "b",
            "antes": { "modo": "cluster", instance_id: "a", "instancias": [null, "x", { id: "a", local: true }], orgs: null },
            "depois": { "modo": "cluster", instance_id: "a", "instancias": [], orgs: { org_1: "a" } },
            "orgs_que_trocaram": [null, { org: "org_1", de: "b", para: null }],
            "instancias_adicionadas": ["c", 7],
            "instancias_removidas": null,
        });

        expect(report["antes"]["instancias"]).toEqual([{ id: "a", local: true }]);
        expect(report["antes"]["quantidadeDeOrgs"]).toBe(0);
        expect(report["orgsQueTrocaram"]).toEqual([{ org: "org_1", de: "b", para: null }]);
        expect(report["instanciasAdicionadas"]).toEqual(["c"]);
        expect(report["instanciasRemovidas"]).toEqual([]);
    });

    it("corpo que não é objeto lança erro de formato", () => {
        expect(() => parseReport(undefined)).toThrow(ApiError);
        expect(() => parseReport({ "mudou": "sim" })).toThrow(ApiError);
    });
});

describe("mensagens em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("saúde com erro fala inglês nos dois casos", async () => {
        setLocale("en-US");
        new FakeServer()["responder"]("GET", "/healthz", apiError(500, "interno", "falha"))["instalar"]();
        await expect(fetchHealth()).rejects.toThrow(/answered 500 to the health request/);

        new FakeServer()["responder"]("GET", "/healthz", { status: 200, "corpo": { status: "ok" } })["instalar"]();
        await expect(fetchHealth()).rejects.toThrow(/answered 200 without the health summary/);
    });

    it("métricas com erro e queda de rede falam inglês", async () => {
        setLocale("en-US");
        new FakeServer()["responder"]("GET", "/metrics", apiError(503, "indisponivel", "falha"))["instalar"]();
        await expect(fetchMetrics()).rejects.toThrow(/answered 503 to the metrics request/);
    });

    it("plugin sem resposta usa a frase de reserva em inglês", async () => {
        setLocale("en-US");
        new FakeServer()["responder"]("GET", "/manifests", { status: 200, "corpo": { items: [{ kind: "eco", situacao: "indisponivel" }] } })["instalar"]();

        const [item] = await listPlugins("org_1");
        expect(item?.mensagem).toMatch(/check the program declared in the instance configuration/);
    });

    it("recarga sem o resumo esperado fala inglês", () => {
        setLocale("en-US");
        expect(() => parseReport({})).toThrow(/without the expected summary/);
    });
});
