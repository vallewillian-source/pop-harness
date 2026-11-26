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
import type { FileOrigin } from "../api/testFile";
import { useI18n } from "../i18n";
import "../estilos/chat-teste.css";

export type PreviaDeImagem = {
    /** De onde vem a imagem: é o que o "Tentar de novo" reabre. */
    origem: FileOrigin;
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
    const { translate: t } = useI18n();
    return (
        <SofaModal
            open={previa !== null}
            onOpenChange={(proximo) => {
                if (!proximo) aoFechar();
            }}
            title={
                previa
                    ? t("testChat.previewTitle", { name: previa.origem.nome })
                    : t("testChat.previewTitlePlain")
            }
            description={
                previa?.origem.de === "agente"
                    ? t("testChat.previewFromAgent")
                    : t("testChat.previewFromClient")
            }
            closeLabel={t("testChat.previewClose")}
            size="lg"
        >
            {previa?.estado === "carregando" && (
                <p className="poph-previa-arquivo__estado" role="status">
                    {t("testChat.previewLoading")}
                </p>
            )}
            {previa?.estado === "erro" && (
                <SofaBanner tone="danger" title={t("testChat.previewErrorTitle")}>
                    <p>{previa.erro}</p>
                    <SofaButton variant="secondary" onClick={aoTentarDeNovo}>
                        {t("testChat.retry")}
                    </SofaButton>
                </SofaBanner>
            )}
            {previa?.estado === "pronta" && previa.src && (
                <img
                    className="poph-previa-arquivo__imagem"
                    src={previa.src}
                    alt={
                        previa.origem.de === "agente"
                            ? t("testChat.imageFromAgent", { name: previa.origem.nome })
                            : t("testChat.imageFromClient", { name: previa.origem.nome })
                    }
                />
            )}
        </SofaModal>
    );
}
