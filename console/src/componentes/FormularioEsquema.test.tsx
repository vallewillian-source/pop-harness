/**
 * Testes do formulário gerado por JSON Schema (HR-44).
 *
 * O que importa: o esquema real do plugin de exemplo (`poph-plugin-eco`, copiado de
 * `crates/poph-plugin-eco/src/main.rs`) vira formulário **sem código por integração**; segredo é
 * campo de senha e nunca aparece com valor; obrigatório e `enum` são validados antes de enviar.
 */

import { useState } from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
    FormularioEsquema,
    camposDoEsquema,
    validarValoresDeEsquema,
    valoresIniciais,
    type EsquemaDeObjeto,
    type ValoresDeEsquema,
} from "./FormularioEsquema";

/** Esquema do manifesto do plugin eco (HR-27), copiado sem alteração. */
const ESQUEMA_ECO: EsquemaDeObjeto = {
    type: "object",
    properties: {
        modo: {
            type: "string",
            enum: ["inverter"],
            default: "inverter",
            description: "O eco só tem o modo que inverte o texto.",
        },
    },
};

/** Um plugin com configuração pública e segredo, no formato do manifesto. */
const ESQUEMA_PLUGIN: EsquemaDeObjeto = {
    type: "object",
    properties: {
        endpoint: { type: "string", description: "URL do serviço do plugin." },
        tentativas: { type: "integer", minimum: 1, maximum: 5, default: 3 },
        avisar: { type: "boolean", default: false },
    },
    required: ["endpoint"],
};
const SEGREDOS_PLUGIN: EsquemaDeObjeto = {
    type: "object",
    properties: {
        api_key: { type: "string", title: "Chave da API", description: "Credencial do plugin." },
    },
    required: ["api_key"],
};

function FormularioDeTeste({
    configuracao,
    segredos = null,
    iniciais,
}: {
    configuracao: EsquemaDeObjeto;
    segredos?: EsquemaDeObjeto | null;
    iniciais?: ValoresDeEsquema;
}) {
    const [valores, setValores] = useState<ValoresDeEsquema>(
        iniciais ?? valoresIniciais(configuracao, segredos),
    );
    return (
        <FormularioEsquema
            configuracao={configuracao}
            segredos={segredos}
            valores={valores}
            idPrefixo="teste"
            aoMudar={(nome, valor) => setValores((atuais) => ({ ...atuais, [nome]: valor }))}
        />
    );
}

describe("FormularioEsquema", () => {
    it("desenha o esquema do plugin de exemplo sem código por integração", () => {
        render(<FormularioDeTeste configuracao={ESQUEMA_ECO} />);

        expect(screen.getByLabelText(/Modo/)).toHaveValue("inverter");
        expect(screen.getByText("O eco só tem o modo que inverte o texto.")).toBeInTheDocument();
        expect(valoresIniciais(ESQUEMA_ECO, null)).toEqual({ modo: "inverter" });
    });

    it("separa configuração pública de segredo, e o segredo é campo de senha", async () => {
        const usuario = userEvent.setup();
        render(<FormularioDeTeste configuracao={ESQUEMA_PLUGIN} segredos={SEGREDOS_PLUGIN} />);

        const chave = screen.getByLabelText(/Chave da API/);
        expect(chave).toHaveAttribute("type", "password");
        expect(chave).toHaveValue("");

        await usuario.type(chave, "segredo-do-teste");
        // Digitado, continua escondido: nenhum texto do segredo aparece na página.
        expect(screen.queryByText("segredo-do-teste")).not.toBeInTheDocument();

        const campos = camposDoEsquema(ESQUEMA_PLUGIN, SEGREDOS_PLUGIN);
        expect(campos.map((campo) => [campo.nome, campo.segredo])).toEqual([
            ["endpoint", false],
            ["tentativas", false],
            ["avisar", false],
            ["api_key", true],
        ]);
    });

    it("exige o campo obrigatório e o segredo obrigatório antes de enviar", () => {
        const erros = validarValoresDeEsquema(ESQUEMA_PLUGIN, SEGREDOS_PLUGIN, {
            endpoint: "",
            tentativas: 3,
            avisar: false,
            api_key: "",
        });

        expect(erros["endpoint"]).toBe("campo obrigatório");
        expect(erros["api_key"]).toBe("informe o segredo");

        const completos = validarValoresDeEsquema(ESQUEMA_PLUGIN, SEGREDOS_PLUGIN, {
            endpoint: "https://plugin.exemplo",
            tentativas: 3,
            avisar: false,
            api_key: "chave",
        });
        expect(completos).toEqual({});
    });

    it("um esquema novo gera o formulário sem tocar no Console", async () => {
        const usuario = userEvent.setup();
        // Plugin "sms" inventado depois: só esquema, nenhuma linha de código por integração.
        const esquemaSms: EsquemaDeObjeto = {
            type: "object",
            properties: {
                remetente: { type: "string", title: "Remetente", description: "Número que envia." },
                prioridade: { type: "string", enum: ["normal", "alta"], default: "normal" },
                copia: { type: "boolean", title: "Enviar cópia ao operador" },
            },
            required: ["remetente"],
        };
        render(<FormularioDeTeste configuracao={esquemaSms} />);

        await usuario.type(screen.getByLabelText(/Remetente/), "5511999999999");
        await usuario.selectOptions(screen.getByLabelText("Prioridade"), "alta");
        await usuario.click(screen.getByLabelText("Enviar cópia ao operador"));

        expect(screen.getByLabelText(/Remetente/)).toHaveValue("5511999999999");
        expect(screen.getByLabelText("Prioridade")).toHaveValue("alta");
        expect(screen.getByLabelText("Enviar cópia ao operador")).toBeChecked();
    });

    it("recusa valor fora do enum e número fora da faixa", () => {
        const foraDoEnum = validarValoresDeEsquema(ESQUEMA_ECO, null, { modo: "outro" });
        expect(foraDoEnum["modo"]).toContain("use um destes valores: inverter");

        const foraDaFaixa = validarValoresDeEsquema(ESQUEMA_PLUGIN, null, {
            endpoint: "https://plugin.exemplo",
            tentativas: 9,
        });
        expect(foraDaFaixa["tentativas"]).toBe("o máximo é 5");

        const fracionado = validarValoresDeEsquema(ESQUEMA_PLUGIN, null, {
            endpoint: "https://plugin.exemplo",
            tentativas: 2.5,
        });
        expect(fracionado["tentativas"]).toBe("informe um número inteiro");
    });

    it("desenha booleano como caixa e avisa quando o esquema não tem campo", async () => {
        const usuario = userEvent.setup();
        render(<FormularioDeTeste configuracao={ESQUEMA_PLUGIN} iniciais={{ endpoint: "", avisar: false }} />);

        const avisar = screen.getByLabelText("Avisar");
        expect(avisar).not.toBeChecked();
        await usuario.click(avisar);
        expect(avisar).toBeChecked();

        render(<FormularioDeTeste configuracao={{ type: "object", properties: {} }} />);
        expect(
            screen.getByText("Esta integração não tem configuração nem segredo no manifesto."),
        ).toBeInTheDocument();
    });
});
