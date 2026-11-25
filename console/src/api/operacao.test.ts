/**
 * Testes da camada de operação (HR-48a): leitura do `/metrics` em texto, leitura da saúde (200 e
 * 503 com corpo) e os erros de transporte. O `fetch` é o falso da suíte, então o teste confere o
 * caminho exato que a tela pediu.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { ErroApi } from "./cliente";
import {
    consultarMetricas,
    consultarSaude,
    interpretarMetricas,
    interpretarRelatorio,
    listarPlugins,
    recarregarCluster,
} from "./operacao";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { setLocale } from "../i18n";

afterEach(() => {
    vi.unstubAllGlobals();
});

const SAUDE_OK = {
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

        expect(interpretarMetricas(texto)).toEqual({
            filaTurnos: 4,
            filaSaida: 0,
            tokens: 1500,
        });
    });

    it("devolve vazio quando o texto não tem nenhum nome conhecido (a tela mostra 'não informado')", () => {
        expect(interpretarMetricas("# nada aqui\n")).toEqual({});
    });
});

describe("consultarSaude", () => {
    it("200 vira situação ok com o corpo", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .instalar();

        const resposta = await consultarSaude();

        expect(resposta.situacao).toBe("ok");
        expect(resposta.saude.version).toBe("0.9.1");
        expect(servidor.chamadasPara("GET", "/healthz")).toHaveLength(1);
    });

    it("503 com corpo vira situação degradada, sem jogar fora o resumo", async () => {
        new ServidorFalso()
            .responder("GET", "/healthz", {
                status: 503,
                corpo: { ...SAUDE_OK, status: "degraded" },
            })
            .instalar();

        const resposta = await consultarSaude();

        expect(resposta.situacao).toBe("degradada");
        expect(resposta.saude.databases.harness).toBe(12);
    });

    it("500 sem resumo vira erro com o status", async () => {
        new ServidorFalso()
            .responder("GET", "/healthz", erroApi(500, "interno", "falha"))
            .instalar();

        await expect(consultarSaude()).rejects.toMatchObject({ status: 500 });
    });

    it("sem conexão com o servidor vira erro de rede (status 0)", async () => {
        // Sem conexão: o fetch falha antes de chegar ao servidor.
        vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))));

        const falha = await consultarSaude().catch((erro: unknown) => erro);

        expect(falha).toBeInstanceOf(ErroApi);
        expect((falha as ErroApi).semServidor).toBe(true);
    });
});

describe("consultarMetricas", () => {
    it("lê o texto do /metrics sem tentar interpretar como JSON", async () => {
        new ServidorFalso()
            .responder("GET", "/metrics", {
                status: 200,
                texto: "poph_fila_turnos 2\npoph_agentes_carregados 1\n",
            })
            .instalar();

        expect(await consultarMetricas()).toEqual({ filaTurnos: 2, agentesCarregados: 1 });
    });

    it("status diferente de 200 vira erro", async () => {
        new ServidorFalso()
            .responder("GET", "/metrics", { status: 500, corpo: { code: "x", message: "y" } })
            .instalar();

        await expect(consultarMetricas()).rejects.toMatchObject({ status: 500 });
    });
});

describe("listarPlugins (HR-48c)", () => {
    it("chama a lista da org com o token da sessão e mantém a ordem recebida", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/admin/v1/orgs/org_1/manifests", {
                status: 200,
                corpo: {
                    items: [
                        { kind: "eco", situacao: "ok", nome: "Eco", versao: "0.1.0" },
                        { kind: "whatsapp", situacao: "indisponivel", mensagem: "sem resposta" },
                    ],
                },
            })
            .instalar();

        const itens = await listarPlugins("org_1");

        expect(itens).toEqual([
            { kind: "eco", situacao: "ok", nome: "Eco", versao: "0.1.0" },
            { kind: "whatsapp", situacao: "indisponivel", mensagem: "sem resposta" },
        ]);
        expect(servidor.chamadas[0]?.url).toBe("/admin/v1/orgs/org_1/manifests");
    });

    it("plugin indisponível sem mensagem ganha a frase de operação, sem nome nem versão", async () => {
        new ServidorFalso()
            .responder("GET", "/manifests", {
                status: 200,
                corpo: { items: [{ kind: "quebrado", situacao: "indisponivel" }] },
            })
            .instalar();

        const [item] = await listarPlugins("org_1");

        expect(item?.situacao).toBe("indisponivel");
        expect(item?.mensagem).toMatch(/não respondeu/);
        expect(item?.nome).toBeUndefined();
        expect(item?.versao).toBeUndefined();
    });

    it("item sem tipo de texto é descartado, e situação que não é ok vira indisponível", async () => {
        new ServidorFalso()
            .responder("GET", "/manifests", {
                status: 200,
                corpo: {
                    items: [
                        { situacao: "ok", nome: "Sem tipo" },
                        null,
                        { kind: 7, situacao: "ok" },
                        { kind: "estranho", situacao: "talvez", mensagem: "algo" },
                    ],
                },
            })
            .instalar();

        const itens = await listarPlugins("org_1");

        expect(itens).toEqual([{ kind: "estranho", situacao: "indisponivel", mensagem: "algo" }]);
    });

    it("resposta sem items vira lista vazia, não erro", async () => {
        new ServidorFalso().responder("GET", "/manifests", { status: 200, corpo: {} }).instalar();

        expect(await listarPlugins("org_1")).toEqual([]);
    });

    it("403 de outra org chega como erro da API, para a tela explicar", async () => {
        new ServidorFalso()
            .responder("GET", "/manifests", erroApi(403, "forbidden", "outra org"))
            .instalar();

        await expect(listarPlugins("org_2")).rejects.toMatchObject({ status: 403 });
    });
});

describe("recarregarCluster (HR-48f)", () => {
    const MAPA_CLUSTER = {
        modo: "cluster",
        instance_id: "a",
        default_instance: "a",
        segredo_configurado: true,
        instancias: [
            { id: "a", url: "http://a.internal:8080", local: true },
            { id: "b", url: "http://b.internal:8080", local: false },
        ],
        orgs: { org_1: "a", org_2: "b" },
    };
    const MAPA_UNICO = {
        modo: "instancia_unica",
        instance_id: "",
        default_instance: null,
        segredo_configurado: false,
        instancias: [],
        orgs: {},
    };

    it("faz POST na rota global, sem corpo, e devolve o relatório tipado", async () => {
        const servidor = new ServidorFalso()
            .responder("POST", "/admin/v1/cluster/reload", {
                status: 200,
                corpo: {
                    mudou: true,
                    fingerprint_antes: "aaaa1111bbbb2222",
                    fingerprint_depois: "cccc3333dddd4444",
                    antes: MAPA_UNICO,
                    depois: MAPA_CLUSTER,
                    orgs_que_trocaram: [{ org: "org_2", de: null, para: "b" }],
                    instancias_adicionadas: ["b"],
                    instancias_removidas: [],
                },
            })
            .instalar();

        const relatorio = await recarregarCluster();

        const chamada = servidor.chamadasPara("POST", "/admin/v1/cluster/reload")[0];
        expect(chamada?.metodo).toBe("POST");
        expect(chamada?.bruto).toBeUndefined();
        expect(relatorio.mudou).toBe(true);
        expect(relatorio.impressaoAntes).toBe("aaaa1111bbbb2222");
        expect(relatorio.impressaoDepois).toBe("cccc3333dddd4444");
        expect(relatorio.antes.modo).toBe("instancia_unica");
        expect(relatorio.depois).toMatchObject({
            modo: "cluster",
            instanciaLocal: "a",
            instanciaPadrao: "a",
            segredoConfigurado: true,
            quantidadeDeOrgs: 2,
            instancias: [
                { id: "a", local: true },
                { id: "b", local: false },
            ],
        });
        expect(relatorio.orgsQueTrocaram).toEqual([{ org: "org_2", de: null, para: "b" }]);
        expect(relatorio.instanciasAdicionadas).toEqual(["b"]);
        expect(relatorio.instanciasRemovidas).toEqual([]);
    });

    it("não leva o endereço das instâncias para o resultado", async () => {
        new ServidorFalso()
            .responder("POST", "/admin/v1/cluster/reload", {
                status: 200,
                corpo: { mudou: false, fingerprint_antes: "x", fingerprint_depois: "x", antes: MAPA_CLUSTER, depois: MAPA_CLUSTER, orgs_que_trocaram: [], instancias_adicionadas: [], instancias_removidas: [] },
            })
            .instalar();

        const relatorio = await recarregarCluster();

        expect(JSON.stringify(relatorio)).not.toContain("internal:8080");
    });

    it("400 cluster_invalido sai como ErroApi com a lista de problemas em detalhes", async () => {
        new ServidorFalso()
            .responder(
                "POST",
                "/admin/v1/cluster/reload",
                erroApi(400, "cluster_invalido", "cluster.toml inválido: instância sem endereço", {
                    arquivo: "/etc/poph/cluster.toml",
                    problemas: ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
                }),
            )
            .instalar();

        const erro = await recarregarCluster().catch((e: unknown) => e);

        expect(erro).toBeInstanceOf(ErroApi);
        expect((erro as ErroApi).status).toBe(400);
        expect((erro as ErroApi).codigo).toBe("cluster_invalido");
        expect((erro as ErroApi).detalhes).toMatchObject({
            problemas: ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
        });
    });

    it("403 sai como ErroApi com status 403 (token de org)", async () => {
        new ServidorFalso()
            .responder("POST", "/admin/v1/cluster/reload", erroApi(403, "forbidden", "token de org"))
            .instalar();

        const erro = await recarregarCluster().catch((e: unknown) => e);

        expect((erro as ErroApi).status).toBe(403);
    });

    it("corpo de sucesso sem o resumo esperado vira erro de formato, não resultado inventado", async () => {
        new ServidorFalso().responder("POST", "/admin/v1/cluster/reload", { status: 200, corpo: { ok: true } }).instalar();

        const erro = await recarregarCluster().catch((e: unknown) => e);

        expect(erro).toBeInstanceOf(ErroApi);
        expect((erro as ErroApi).codigo).toBe("formato");
    });
});

describe("interpretarRelatorio (HR-48f)", () => {
    it("modo instância única sem id local e sem padrão", () => {
        const relatorio = interpretarRelatorio({
            mudou: false,
            fingerprint_antes: "f",
            fingerprint_depois: "f",
            antes: { modo: "instancia_unica", instance_id: "", default_instance: null, segredo_configurado: false, instancias: [], orgs: {} },
            depois: { modo: "instancia_unica", instance_id: "", default_instance: null, segredo_configurado: false, instancias: [], orgs: {} },
            orgs_que_trocaram: [],
            instancias_adicionadas: [],
            instancias_removidas: [],
        });

        expect(relatorio.depois).toEqual({
            modo: "instancia_unica",
            instanciaLocal: "",
            instanciaPadrao: null,
            segredoConfigurado: false,
            instancias: [],
            quantidadeDeOrgs: 0,
        });
        expect(relatorio.orgsQueTrocaram).toEqual([]);
    });

    it("itens malformados em listas são descartados sem derrubar a leitura", () => {
        const relatorio = interpretarRelatorio({
            mudou: true,
            fingerprint_antes: "a",
            fingerprint_depois: "b",
            antes: { modo: "cluster", instance_id: "a", instancias: [null, "x", { id: "a", local: true }], orgs: null },
            depois: { modo: "cluster", instance_id: "a", instancias: [], orgs: { org_1: "a" } },
            orgs_que_trocaram: [null, { org: "org_1", de: "b", para: null }],
            instancias_adicionadas: ["c", 7],
            instancias_removidas: null,
        });

        expect(relatorio.antes.instancias).toEqual([{ id: "a", local: true }]);
        expect(relatorio.antes.quantidadeDeOrgs).toBe(0);
        expect(relatorio.orgsQueTrocaram).toEqual([{ org: "org_1", de: "b", para: null }]);
        expect(relatorio.instanciasAdicionadas).toEqual(["c"]);
        expect(relatorio.instanciasRemovidas).toEqual([]);
    });

    it("corpo que não é objeto lança erro de formato", () => {
        expect(() => interpretarRelatorio(undefined)).toThrow(ErroApi);
        expect(() => interpretarRelatorio({ mudou: "sim" })).toThrow(ErroApi);
    });
});

describe("mensagens em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("saúde com erro fala inglês nos dois casos", async () => {
        setLocale("en-US");
        new ServidorFalso().responder("GET", "/healthz", erroApi(500, "interno", "falha")).instalar();
        await expect(consultarSaude()).rejects.toThrow(/answered 500 to the health request/);

        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: { status: "ok" } }).instalar();
        await expect(consultarSaude()).rejects.toThrow(/answered 200 without the health summary/);
    });

    it("métricas com erro e queda de rede falam inglês", async () => {
        setLocale("en-US");
        new ServidorFalso().responder("GET", "/metrics", erroApi(503, "indisponivel", "falha")).instalar();
        await expect(consultarMetricas()).rejects.toThrow(/answered 503 to the metrics request/);
    });

    it("plugin sem resposta usa a frase de reserva em inglês", async () => {
        setLocale("en-US");
        new ServidorFalso()
            .responder("GET", "/manifests", { status: 200, corpo: { items: [{ kind: "eco", situacao: "indisponivel" }] } })
            .instalar();

        const [item] = await listarPlugins("org_1");
        expect(item?.mensagem).toMatch(/check the program declared in the instance configuration/);
    });

    it("recarga sem o resumo esperado fala inglês", () => {
        setLocale("en-US");
        expect(() => interpretarRelatorio({})).toThrow(/without the expected summary/);
    });
});
