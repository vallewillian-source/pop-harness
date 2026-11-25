/**
 * Formulário gerado a partir de JSON Schema (HR-44 e HR-44e).
 *
 * É a peça que transforma o esquema do manifesto de qualquer plugin em campos, **sem código por
 * integração**: o mesmo componente desenha o formulário do plugin de exemplo (`poph-plugin-eco`) e
 * o de um plugin com segredo. Subconjunto de JSON Schema que o manifesto usa hoje (`poph-types`
 * `EsquemaJson`): `object` com `properties` de `string` (inclusive `enum`), `number`/`integer` e
 * `boolean`, mais `required`, `default`, `title`, `description` e `format: "password"`.
 *
 * A fonte do esquema é a rota do manifesto (`GET …/manifests/{kind}`, HR-44c); o modal da aba
 * Integrações passa `configuracao` e `segredos` dela para cá (HR-44e).
 *
 * Segredo nunca volta para a tela: campo de segredo é `type="password"`, começa vazio mesmo que o
 * manifesto traga `default`, e o valor só é enviado quando o operador digita algo. O que a tela sabe
 * de um segredo já gravado são só os **nomes** (`segredos_preenchidos`) — ou que existe algum, quando
 * a chave de cifra atual não abre o blob (aí a lista vem `null`).
 */

import {
    SofaCheckbox,
    SofaField,
    SofaInput,
    SofaSelect,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { translate, useI18n } from "../i18n";

/** Propriedade de um esquema de objeto (o subconjunto que o manifesto usa). */
export type PropriedadeDeEsquema = {
    type?: string;
    title?: string;
    description?: string;
    format?: string;
    enum?: unknown[];
    default?: unknown;
    minimum?: number;
    maximum?: number;
};

/** Esquema de objeto de um manifesto (`Manifesto.configuracao` / `Manifesto.segredos`). */
export type EsquemaDeObjeto = {
    type?: string;
    properties?: Record<string, PropriedadeDeEsquema>;
    required?: string[];
};

/** Valores do formulário: um por campo, na ordem do esquema. */
export type ValoresDeEsquema = Record<string, unknown>;

/** Campo pronto para desenhar. */
type Campo = {
    nome: string;
    propriedade: PropriedadeDeEsquema;
    obrigatorio: boolean;
    segredo: boolean;
};

/** O que a tela sabe dos segredos já gravados (nunca os valores). */
export type EstadoDosSegredos = {
    /** Nomes já preenchidos; `null` = existe segredo que a chave de cifra atual não abre. */
    preenchidos?: string[] | null;
    /** Nomes que o operador marcou para remover ao salvar (`""` no corpo). */
    removidos?: string[];
};

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Rótulo do campo: `title` do esquema ou o nome com a primeira letra maiúscula. */
export function rotuloDoCampo(nome: string, propriedade: PropriedadeDeEsquema): string {
    if (propriedade.title && propriedade.title.trim() !== "") return propriedade.title;
    const espacado = nome.replace(/[_-]+/g, " ").trim();
    return espacado.charAt(0).toUpperCase() + espacado.slice(1);
}

/**
 * Lista os campos do formulário: primeiro a configuração pública, depois os segredos (que vêm do
 * esquema `segredos` do manifesto). Nome repetido nos dois fica como segredo — o mais restritivo.
 */
export function camposDoEsquema(
    configuracao: EsquemaDeObjeto | null | undefined,
    segredos: EsquemaDeObjeto | null | undefined,
): Campo[] {
    const publicos = Object.entries(configuracao?.properties ?? {}).map(([nome, propriedade]) => ({
        nome,
        propriedade,
        obrigatorio: (configuracao?.required ?? []).includes(nome),
        segredo: propriedade.format === "password",
    }));
    const nomes = new Set(publicos.map((campo) => campo.nome));
    const ocultos = Object.entries(segredos?.properties ?? {})
        .filter(([nome]) => !nomes.has(nome))
        .map(([nome, propriedade]) => ({
            nome,
            propriedade,
            obrigatorio: (segredos?.required ?? []).includes(nome),
            segredo: true,
        }));
    return [...publicos, ...ocultos];
}

/** Valores iniciais: `default` de cada campo público; segredo sempre vazio. */
export function valoresIniciais(
    configuracao: EsquemaDeObjeto,
    segredos: EsquemaDeObjeto | null,
): ValoresDeEsquema {
    const valores: ValoresDeEsquema = {};
    for (const campo of camposDoEsquema(configuracao, segredos)) {
        valores[campo.nome] = campo.segredo ? "" : (campo.propriedade.default ?? "");
    }
    return valores;
}

function vazio(valor: unknown): boolean {
    return (
        valor === undefined || valor === null || (typeof valor === "string" && valor.trim() === "")
    );
}

/**
 * Valida os valores contra o esquema: obrigatórios preenchidos e `enum` respeitado. Devolve uma
 * mensagem por campo; vazio = pode enviar. Segredo obrigatório e vazio pede o valor e um booleano
 * obrigatório precisa estar marcado.
 *
 * `estadoDosSegredos` evita cobrar um segredo que **já está gravado** (a API só manda os nomes) e
 * aceita a remoção marcada pelo operador.
 */
export function validarValoresDeEsquema(
    configuracao: EsquemaDeObjeto | null | undefined,
    segredos: EsquemaDeObjeto | null,
    valores: ValoresDeEsquema,
    estadoDosSegredos: EstadoDosSegredos = {},
): Record<string, string> {
    const erros: Record<string, string> = {};
    const preenchidos = estadoDosSegredos.preenchidos ?? [];
    const removidos = estadoDosSegredos.removidos ?? [];
    for (const campo of camposDoEsquema(configuracao, segredos)) {
        const valor = valores[campo.nome];
        const jaGravado =
            campo.segredo && preenchidos.includes(campo.nome) && !removidos.includes(campo.nome);
        if (
            campo.obrigatorio &&
            !jaGravado &&
            (campo.propriedade.type === "boolean" ? valor !== true : vazio(valor))
        ) {
            erros[campo.nome] = campo.segredo
                ? translate("integrations.schemaValidationSecretRequired")
                : translate("integrations.schemaValidationFieldRequired");
            continue;
        }
        const opcoes = campo.propriedade.enum;
        if (opcoes && !vazio(valor) && !opcoes.some((opcao) => String(opcao) === String(valor))) {
            erros[campo.nome] = translate("integrations.schemaValidationEnum", {
                options: opcoes.map(String).join(", "),
            });
            continue;
        }
        const numerico =
            campo.propriedade.type === "number" || campo.propriedade.type === "integer";
        if (numerico && !vazio(valor)) {
            const numero = Number(valor);
            if (Number.isNaN(numero)) {
                erros[campo.nome] = translate("integrations.schemaValidationNumber");
                continue;
            }
            if (campo.propriedade.type === "integer" && !Number.isInteger(numero)) {
                erros[campo.nome] = translate("integrations.schemaValidationInteger");
                continue;
            }
            if (campo.propriedade.minimum !== undefined && numero < campo.propriedade.minimum) {
                erros[campo.nome] = translate("integrations.schemaValidationMin", {
                    min: campo.propriedade.minimum,
                });
                continue;
            }
            if (campo.propriedade.maximum !== undefined && numero > campo.propriedade.maximum) {
                erros[campo.nome] = translate("integrations.schemaValidationMax", {
                    max: campo.propriedade.maximum,
                });
            }
        }
    }
    return erros;
}

/**
 * Converte os valores do formulário no `config_json` que a API valida.
 *
 * O servidor cobra o **tipo** de cada campo (`string`, `number`, `integer`, `boolean`), e o
 * formulário trabalha com texto: aqui o número vira número e o booleano vira booleano. Campo vazio
 * fica fora do objeto — uma string vazia não serve para `required`, e um número vazio não é número.
 * Booleano sai sempre (o `false` é escolha do operador, não ausência).
 */
export function valoresParaConfiguracao(
    configuracao: EsquemaDeObjeto,
    valores: ValoresDeEsquema,
): Record<string, unknown> {
    const configuracaoEnviada: Record<string, unknown> = {};
    for (const campo of camposDoEsquema(configuracao, null)) {
        if (campo.segredo) continue;
        const valor = valores[campo.nome];
        if (campo.propriedade.type === "boolean") {
            configuracaoEnviada[campo.nome] = valor === true;
            continue;
        }
        if (vazio(valor)) continue;
        if (campo.propriedade.type === "number" || campo.propriedade.type === "integer") {
            const numero = Number(valor);
            if (Number.isFinite(numero)) configuracaoEnviada[campo.nome] = numero;
            continue;
        }
        configuracaoEnviada[campo.nome] = String(valor);
    }
    return configuracaoEnviada;
}

/**
 * Converte os valores do formulário no `secrets` que a API grava (nome → valor).
 *
 * Só entram os segredos que o operador **digitou** nesta sessão — um campo em branco mantém o que já
 * estava gravado. Segredo marcado para remoção vai como `""`, que é como a API apaga.
 */
export function valoresParaSegredos(
    segredos: EsquemaDeObjeto | null,
    valores: ValoresDeEsquema,
    removidos: string[] = [],
): Record<string, string> {
    const enviados: Record<string, string> = {};
    for (const nome of Object.keys(segredos?.properties ?? {})) {
        if (removidos.includes(nome)) {
            enviados[nome] = "";
            continue;
        }
        const valor = valores[nome];
        if (vazio(valor)) continue;
        enviados[nome] = String(valor);
    }
    return enviados;
}

/** Valores iniciais de uma integração que já existe: `default` do esquema + o que está gravado. */
export function valoresDaIntegracao(
    configuracao: EsquemaDeObjeto,
    segredos: EsquemaDeObjeto | null,
    gravados: Record<string, unknown> | null | undefined,
): ValoresDeEsquema {
    return { ...valoresIniciais(configuracao, segredos), ...(gravados ?? {}) };
}

export type FormularioEsquemaProps = {
    configuracao: EsquemaDeObjeto;
    segredos?: EsquemaDeObjeto | null;
    valores: ValoresDeEsquema;
    aoMudar: (nome: string, valor: unknown) => void;
    erros?: Record<string, string>;
    /** Prefixo dos `id` dos campos, para dois formulários na mesma página não colidirem. */
    idPrefixo: string;
    /** Só leitura: desenha os campos sem permitir edição (ex.: antes de o manifesto chegar). */
    somenteLeitura?: boolean;
    /** O que já está gravado nos segredos (nomes, nunca valores). */
    estadoDosSegredos?: EstadoDosSegredos;
    /** Marca/desmarca a remoção de um segredo já gravado. Sem ele não há como remover. */
    aoAlternarRemocao?: (nome: string, remover: boolean) => void;
};

export function FormularioEsquema({
    configuracao,
    segredos = null,
    valores,
    aoMudar,
    erros = {},
    idPrefixo,
    somenteLeitura = false,
    estadoDosSegredos = {},
    aoAlternarRemocao,
}: FormularioEsquemaProps) {
    const { translate: t } = useI18n();
    const campos = camposDoEsquema(configuracao, segredos);
    const preenchidos = estadoDosSegredos.preenchidos ?? [];
    const removidos = estadoDosSegredos.removidos ?? [];
    if (campos.length === 0) {
        return (
            <p className="poph-mono-mudo" style={{ margin: 0 }}>
                {t("integrations.schemaEmptyManifest")}
            </p>
        );
    }

    return (
        <div className="poph-pilha">
            {campos.map((campo) => {
                const id = `${idPrefixo}-${campo.nome}`;
                const rotulo = rotuloDoCampo(campo.nome, campo.propriedade);
                const dica = campo.propriedade.description;
                const erro = erros[campo.nome];
                const valor = valores[campo.nome];
                const opcoes = campo.propriedade.enum;
                const jaGravado = campo.segredo && preenchidos.includes(campo.nome);
                const ilegivel = campo.segredo && estadoDosSegredos.preenchidos === null;
                const removendo = campo.segredo && removidos.includes(campo.nome);
                const dicaDoSegredo = !campo.segredo
                    ? dica
                    : removendo
                      ? t("integrations.schemaSecretRemovingHint")
                      : jaGravado
                        ? t("integrations.schemaSecretSavedHint")
                        : ilegivel
                          ? t("integrations.schemaSecretUnreadableHint")
                          : dica;

                if (campo.propriedade.type === "boolean") {
                    return (
                        <SofaCheckbox
                            key={campo.nome}
                            label={rotulo}
                            description={dica}
                            checked={valor === true}
                            disabled={somenteLeitura}
                            error={!!erro}
                            onChange={(evento) => aoMudar(campo.nome, evento.target.checked)}
                        />
                    );
                }

                return (
                    <div key={campo.nome} className="poph-pilha" style={{ gap: 4 }}>
                        <SofaField
                            label={rotulo}
                            hint={dicaDoSegredo}
                            error={erro}
                            required={campo.obrigatorio && !jaGravado}
                            htmlFor={id}
                        >
                            {(controle) => {
                                const ligacoes = semDescribedBy(controle);
                                if (opcoes) {
                                    return (
                                        <SofaSelect
                                            {...ligacoes}
                                            disabled={somenteLeitura}
                                            value={typeof valor === "string" ? valor : ""}
                                            onChange={(evento) =>
                                                aoMudar(campo.nome, evento.target.value)
                                            }
                                        >
                                            <option value="">{t("integrations.schemaSelectChoose")}</option>
                                            {opcoes.map((opcao) => (
                                                <option key={String(opcao)} value={String(opcao)}>
                                                    {String(opcao)}
                                                </option>
                                            ))}
                                        </SofaSelect>
                                    );
                                }
                                const numerico =
                                    campo.propriedade.type === "number" ||
                                    campo.propriedade.type === "integer";
                                return (
                                    <SofaInput
                                        {...ligacoes}
                                        type={
                                            campo.segredo ? "password" : numerico ? "number" : "text"
                                        }
                                        inputMode={numerico ? "decimal" : undefined}
                                        autoComplete={campo.segredo ? "new-password" : "off"}
                                        disabled={somenteLeitura || removendo}
                                        placeholder={
                                            campo.segredo
                                                ? jaGravado
                                                    ? t("integrations.schemaSecretSavedPlaceholder")
                                                    : ilegivel
                                                      ? t("integrations.schemaSecretUnreadablePlaceholder")
                                                      : t("integrations.schemaSecretDefaultPlaceholder")
                                                : undefined
                                        }
                                        value={
                                            valor === undefined || valor === null
                                                ? ""
                                                : String(valor)
                                        }
                                        onChange={(evento) =>
                                            aoMudar(campo.nome, evento.target.value)
                                        }
                                    />
                                );
                            }}
                        </SofaField>
                        {campo.segredo && jaGravado && !campo.obrigatorio && aoAlternarRemocao && (
                            <SofaCheckbox
                                label={t("integrations.schemaSecretRemoveCheckbox")}
                                checked={removendo}
                                onChange={(evento) =>
                                    aoAlternarRemocao(campo.nome, evento.target.checked)
                                }
                            />
                        )}
                        {campo.segredo && jaGravado && campo.obrigatorio && (
                            <span className="poph-mono-mudo">
                                {t("integrations.schemaSecretRequiredNote")}
                            </span>
                        )}
                        {campo.segredo && !jaGravado && ilegivel && !somenteLeitura && (
                            <span className="poph-mono-mudo">
                                {t("integrations.schemaSecretUnreadableNote")}
                            </span>
                        )}
                    </div>
                );
            })}
        </div>
    );
}
