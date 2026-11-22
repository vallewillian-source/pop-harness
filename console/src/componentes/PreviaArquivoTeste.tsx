/**
 * Prévia de imagem do chat de teste (HR-46c; HR-46e para o arquivo do agente).
 *
 * Abre centralizada sobre a conversa (`SofaModal` tamanho `lg`: foco preso, `Esc` fecha, foco volta
 * ao botão "Ver <arquivo>"). A imagem cabe em 80% da largura e da altura da janela (HR-49b: a gaveta
 * lateral espremia a imagem). Mostra "carregando", a imagem, ou o erro com "Tentar de novo".
 *
 * PDF e Office não passam por aqui: são só baixados (ver `AgenteChatTeste.tsx`).
 */

import { SofaBanner, SofaButton, SofaModal } from "@sofacoding/sofa-ui";
import type { OrigemDeArquivo } from "../api/arquivoTeste";
import "../estilos/chat-teste.css";

export type PreviaDeImagem = {
    /** De onde vem a imagem: é o que o "Tentar de novo" reabre. */
    origem: OrigemDeArquivo;
    estado: "carregando" | "pronta" | "erro";
    /** Imagem em `data:` quando `estado` é `pronta`. */
    src: string | null;
    /** Motivo do erro, quando `estado` é `erro`. */
    erro: string | null;
};

type Props = {
    previa: PreviaDeImagem | null;
    aoFechar: () => void;
    aoTentarDeNovo: () => void;
};

export function PreviaArquivoTeste({ previa, aoFechar, aoTentarDeNovo }: Props) {
    return (
        <SofaModal
            open={previa !== null}
            onOpenChange={(proximo) => {
                if (!proximo) aoFechar();
            }}
            title={previa ? `Imagem: ${previa.origem.nome}` : "Imagem"}
            description={
                previa?.origem.de === "agente"
                    ? "Arquivo que o agente enviou nesta conversa de teste."
                    : "Arquivo enviado pelo cliente nesta conversa de teste."
            }
            closeLabel="Fechar a prévia da imagem"
            size="lg"
        >
            {previa?.estado === "carregando" && (
                <p className="poph-previa-arquivo__estado" role="status">
                    Carregando a imagem…
                </p>
            )}
            {previa?.estado === "erro" && (
                <SofaBanner tone="danger" title="Não consegui abrir a imagem">
                    <p>{previa.erro}</p>
                    <SofaButton variant="secondary" onClick={aoTentarDeNovo}>
                        Tentar de novo
                    </SofaButton>
                </SofaBanner>
            )}
            {previa?.estado === "pronta" && previa.src && (
                <img
                    className="poph-previa-arquivo__imagem"
                    src={previa.src}
                    alt={`Imagem enviada pelo ${previa.origem.de === "agente" ? "agente" : "cliente"}: ${previa.origem.nome}`}
                />
            )}
        </SofaModal>
    );
}
