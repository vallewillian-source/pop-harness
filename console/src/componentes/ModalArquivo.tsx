/**
 * Envio de arquivo para a biblioteca do agente (HR-43), em modal.
 *
 * O limite de 16 MB é conferido **ao escolher** o arquivo: a mensagem sai com o tamanho e o limite,
 * e o botão de enviar não chama a API. Se o servidor ainda assim recusar (tipo não aceito, arquivo
 * vazio, corpo acima do limite), o motivo que ele devolve aparece no campo Arquivo, como veio.
 *
 * Não há `If-Match` aqui: a biblioteca não tem versão.
 */

import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaTextarea,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import {
    LIMITE_ARQUIVO_BYTES,
    LIMITE_DESCRICAO_ARQUIVO,
    LIMITE_NOME_ARQUIVO,
    enviarArquivo,
    tamanhoEmMb,
    type Arquivo,
} from "../api/arquivos";

/** Tipos que o servidor aceita (docs/BIBLIOTECA_ARQUIVOS.md e docs/ARQUIVOS.md). */
const TIPOS_ACEITOS =
    ".pdf,.docx,.xlsx,.xls,.pptx,image/jpeg,image/png,image/webp,image/gif";

export type ModalArquivoProps = {
    org: string;
    agente: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Chamado com o arquivo devolvido pela API, já na biblioteca. */
    aoEnviar: (arquivo: Arquivo) => void;
};

type Erros = { arquivo?: string; nome?: string; descricao?: string };

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Quanto o arquivo passa do limite, em KB (abaixo de 1 MB) ou MB. */
function excessoLegivel(bytes: number): string {
    const excesso = bytes - LIMITE_ARQUIVO_BYTES;
    if (excesso < 1024 * 1024) {
        return `${Math.max(1, Math.ceil(excesso / 1024))} KB`;
    }
    return tamanhoEmMb(excesso);
}

/** Mensagem de tamanho para o arquivo escolhido, ou `null` se cabe no limite. */
export function erroDeTamanho(bytes: number): string | null {
    if (bytes === 0) return "o arquivo está vazio";
    if (bytes > LIMITE_ARQUIVO_BYTES) {
        return `o arquivo passa do limite de 16 MB por arquivo em ${excessoLegivel(bytes)}. Reduza ou comprima o arquivo antes de enviar.`;
    }
    return null;
}

export function ModalArquivo({ org, agente, aberto, aoFechar, aoEnviar }: ModalArquivoProps) {
    const [escolhido, setEscolhido] = useState<File | null>(null);
    const [nome, setNome] = useState("");
    const [descricao, setDescricao] = useState("");
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setEscolhido(null);
        setNome("");
        setDescricao("");
        setErros({});
        setErroGeral(null);
        setEnviando(false);
    }, [aberto]);

    function aoEscolher(arquivo: File | null) {
        setEscolhido(arquivo);
        setErroGeral(null);
        if (!arquivo) {
            setErros((atuais) => ({ ...atuais, arquivo: undefined }));
            return;
        }
        setErros((atuais) => ({ ...atuais, arquivo: erroDeTamanho(arquivo.size) ?? undefined }));
    }

    function validar(): Erros {
        const novos: Erros = {};
        if (!escolhido) {
            novos.arquivo = "escolha o arquivo a enviar";
        } else {
            const tamanho = erroDeTamanho(escolhido.size);
            if (tamanho) novos.arquivo = tamanho;
        }
        if (nome.trim().length > LIMITE_NOME_ARQUIVO) {
            novos.nome = `o nome aceita até ${LIMITE_NOME_ARQUIVO} caracteres`;
        }
        if (descricao.trim().length > LIMITE_DESCRICAO_ARQUIVO) {
            novos.descricao = `a descrição aceita até ${LIMITE_DESCRICAO_ARQUIVO} caracteres`;
        }
        return novos;
    }

    async function enviar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validar();
        setErros(novos);
        setErroGeral(null);
        if (Object.keys(novos).length > 0 || !escolhido) return;

        setEnviando(true);
        try {
            const criado = await enviarArquivo(org, agente, escolhido, nome, descricao);
            aoEnviar(criado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ErroApi) {
                // O servidor diz o motivo em `details.reason` (tipo, vazio, tamanho); a mensagem
                // genérica ("campo arquivo inválido") fica de lado quando o motivo existe.
                const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
                const motivo =
                    typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : null;
                if (motivo && falha.status === 400) {
                    setErros({ arquivo: motivo });
                    return;
                }
                setErroGeral(`${falha.message} (código ${falha.codigo})`);
                return;
            }
            setErroGeral("falha inesperada ao enviar o arquivo");
        } finally {
            setEnviando(false);
        }
    }

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title="Enviar arquivo ao agente"
            description="O agente só envia ao cliente o que estiver nesta biblioteca. Ele escolhe pelo nome e pela descrição."
            closeLabel="Fechar envio de arquivo"
            size="md"
            id="poph-formulario-arquivo"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        Cancelar
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void enviar()}
                        loading={enviando}
                    >
                        Enviar arquivo
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-arquivo"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void enviar(evento)}
                noValidate
            >
                {erroGeral && (
                    <SofaBanner tone="danger" title="Não foi possível enviar">
                        {erroGeral}
                    </SofaBanner>
                )}

                <SofaField
                    label="Arquivo"
                    hint={`PDF, Word (.docx), Excel (.xlsx ou .xls), PowerPoint (.pptx) ou imagem (JPEG, PNG, WebP, GIF). Até 16 MB.`}
                    error={erros.arquivo}
                    required
                    htmlFor="poph-arquivo-escolha"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            type="file"
                            accept={TIPOS_ACEITOS}
                            invalid={Boolean(erros.arquivo)}
                            onChange={(evento) =>
                                aoEscolher(evento.target.files?.[0] ?? null)
                            }
                        />
                    )}
                </SofaField>
                {escolhido && !erros.arquivo && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        Escolhido: {escolhido.name} · {tamanhoEmMb(escolhido.size)} (dentro do
                        limite de 16 MB)
                    </p>
                )}

                <SofaField
                    label="Nome mostrado ao cliente"
                    hint={`Opcional. Se ficar vazio, usa o nome do arquivo. Até ${LIMITE_NOME_ARQUIVO} caracteres.`}
                    error={erros.nome}
                    htmlFor="poph-arquivo-nome"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={nome}
                            autoComplete="off"
                            onChange={(evento) => setNome(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label="Descrição para o agente"
                    hint={`Opcional, mas é o que o agente lê para decidir quando enviar. Até ${LIMITE_DESCRICAO_ARQUIVO} caracteres.`}
                    error={erros.descricao}
                    htmlFor="poph-arquivo-descricao"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={descricao}
                            rows={3}
                            onChange={(evento) => setDescricao(evento.target.value)}
                        />
                    )}
                </SofaField>
            </form>
        </SofaModal>
    );
}
