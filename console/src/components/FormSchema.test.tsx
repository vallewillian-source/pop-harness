
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { I18nProvider, setLocale } from "../i18n";

afterEach(() => {
    setLocale("pt-BR");
});
import {
    FormSchema,
    fieldsSchema,
    validateValuesSchema,
    valuesInitials,
    type SchemaObject,
    type ValuesSchema,
} from "../components/FormSchema";

const SCHEMA_ECO: SchemaObject = {
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

const SCHEMA_PLUGIN: SchemaObject = {
    type: "object",
    properties: {
        endpoint: { type: "string", description: "URL do serviço do plugin." },
        tentativas: { type: "integer", minimum: 1, maximum: 5, default: 3 },
        notify: { type: "boolean", default: false },
    },
    required: ["endpoint"],
};
const SECRETS_PLUGIN: SchemaObject = {
    type: "object",
    properties: {
        api_key: { type: "string", title: "Chave da API", description: "Credencial do plugin." },
    },
    required: ["api_key"],
};

function FormTest({
    configuracao,
    segredos = null,
    iniciais,
}: {
    configuracao: SchemaObject;
    segredos?: SchemaObject | null;
    iniciais?: ValuesSchema;
}) {
    const [values, setValues] = useState<ValuesSchema>(
        iniciais ?? valuesInitials(configuracao, segredos),
    );
    return (
        <FormSchema
            configuracao={configuracao}
            segredos={segredos}
            values={values}
            idPrefix="teste"
            onChange={(nome, value) => setValues((current) => ({ ...current, [nome]: value }))}
        />
    );
}

describe("FormularioEsquema", () => {
    it("desenha o esquema do plugin de exemplo sem código por integração", () => {
        render(<FormTest configuracao={SCHEMA_ECO} />);

        expect(screen.getByLabelText(/Modo/)).toHaveValue("inverter");
        expect(screen.getByText("O eco só tem o modo que inverte o texto.")).toBeInTheDocument();
        expect(valuesInitials(SCHEMA_ECO, null)).toEqual({ modo: "inverter" });
    });

    it("separa configuração pública de segredo, e o segredo é campo de senha", async () => {
        const user = userEvent.setup();
        render(<FormTest configuracao={SCHEMA_PLUGIN} segredos={SECRETS_PLUGIN} />);

        const chave = screen.getByLabelText(/Chave da API/);
        expect(chave).toHaveAttribute("type", "password");
        expect(chave).toHaveValue("");

        await user.type(chave, "segredo-do-teste");
        expect(screen.queryByText("segredo-do-teste")).not.toBeInTheDocument();

        const campos = fieldsSchema(SCHEMA_PLUGIN, SECRETS_PLUGIN);
        expect(campos.map((campo) => [campo.nome, campo.secret])).toEqual([
            ["endpoint", false],
            ["tentativas", false],
            ["notify", false],
            ["api_key", true],
        ]);
    });

    it("exige o campo obrigatório e o segredo obrigatório antes de enviar", () => {
        const errors = validateValuesSchema(SCHEMA_PLUGIN, SECRETS_PLUGIN, {
            endpoint: "",
            tentativas: 3,
            notify: false,
            api_key: "",
        });

        expect(errors["endpoint"]).toBe("campo obrigatório");
        expect(errors["api_key"]).toBe("informe o segredo");

        const completos = validateValuesSchema(SCHEMA_PLUGIN, SECRETS_PLUGIN, {
            endpoint: "https://plugin.exemplo",
            tentativas: 3,
            notify: false,
            api_key: "chave",
        });
        expect(completos).toEqual({});
    });

    it("um esquema novo gera o formulário sem tocar no Console", async () => {
        const user = userEvent.setup();
        const schemaSms: SchemaObject = {
            type: "object",
            properties: {
                remetente: { type: "string", title: "Remetente", description: "Número que envia." },
                prioridade: { type: "string", enum: ["normal", "alta"], default: "normal" },
                copy: { type: "boolean", title: "Enviar cópia ao operador" },
            },
            required: ["remetente"],
        };
        render(<FormTest configuracao={schemaSms} />);

        await user.type(screen.getByLabelText(/Remetente/), "5511999999999");
        await user.selectOptions(screen.getByLabelText("Prioridade"), "alta");
        await user.click(screen.getByLabelText("Enviar cópia ao operador"));

        expect(screen.getByLabelText(/Remetente/)).toHaveValue("5511999999999");
        expect(screen.getByLabelText("Prioridade")).toHaveValue("alta");
        expect(screen.getByLabelText("Enviar cópia ao operador")).toBeChecked();
    });

    it("recusa valor fora do enum e número fora da faixa", () => {
        const outsideEnum = validateValuesSchema(SCHEMA_ECO, null, { modo: "outro" });
        expect(outsideEnum["modo"]).toContain("use um destes valores: inverter");

        const outsideRange = validateValuesSchema(SCHEMA_PLUGIN, null, {
            endpoint: "https://plugin.exemplo",
            tentativas: 9,
        });
        expect(outsideRange["tentativas"]).toBe("o máximo é 5");

        const fracionado = validateValuesSchema(SCHEMA_PLUGIN, null, {
            endpoint: "https://plugin.exemplo",
            tentativas: 2.5,
        });
        expect(fracionado["tentativas"]).toBe("informe um número inteiro");
    });

    it("desenha booleano como caixa e avisa quando o esquema não tem campo", async () => {
        const user = userEvent.setup();
        render(<FormTest configuracao={SCHEMA_PLUGIN} iniciais={{ endpoint: "", notify: false }} />);

        const notify = screen.getByLabelText("Notify");
        expect(notify).not.toBeChecked();
        await user.click(notify);
        expect(notify).toBeChecked();

        render(<FormTest configuracao={{ type: "object", properties: {} }} />);
        expect(
            screen.getByText("Esta integração não tem configuração nem segredo no manifesto."),
        ).toBeInTheDocument();
    });

    it("valida e renderiza textos em inglês quando en-US está ativo", () => {
        setLocale("en-US");
        const errors = validateValuesSchema(SCHEMA_PLUGIN, SECRETS_PLUGIN, {
            endpoint: "",
            tentativas: 9,
            notify: false,
            api_key: "",
        });
        expect(errors["endpoint"]).toBe("required field");
        expect(errors["api_key"]).toBe("enter the secret");
        expect(errors["tentativas"]).toBe("maximum is 5");

        render(
            <I18nProvider>
                <FormTest configuracao={{ type: "object", properties: {} }} />
            </I18nProvider>,
        );
        expect(
            screen.getByText("This integration has no configuration or secrets in the manifest."),
        ).toBeInTheDocument();
        setLocale("pt-BR");
    });
});
