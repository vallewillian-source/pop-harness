/**
 * Seção "Exportar e importar a organização" da tela de operação (HR-48e-2).
 *
 * Duas ações explícitas do operador, nunca disparadas pela atualização de 15 s da tela:
 * - Exportar: baixa o arquivo da organização (com ou sem conversas), com o nome que o servidor manda.
 * - Importar: escolhe um arquivo, confere o tamanho (teto de 64 MB) antes de sair da tela, pede
 *   confirmação e só então envia o texto do arquivo para esta organização. Importar não apaga o que
 *   já existe aqui; se houver conflito de nome, o servidor recusa antes de gravar.
 *
 * As rotas e a tradução de erros estão em `api/backup.ts`. Nada de segredo ou caminho aparece aqui.
 */

import { useId, useState } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaCheckbox,
    SofaConfirmDialog,
    SofaField,
    SofaInput,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import {
    baixarArquivo,
    exportarOrganizacao,
    falhaDeBackup,
    importarOrganizacao,
    lerTextoDoArquivo,
    LIMITE_IMPORTACAO_BYTES,
    nomeDaTabela,
    passaDoLimite,
    recusaDeSobrescrita,
    tamanhoEmMb,
    textoDeExemplo,
    type FalhaDeBackup,
    type RelatorioImportacao,
    type RecusaSobrescrita,
} from "../api/backup";

/** Teto da importação em MB inteiros, para as mensagens ("64 MB"). */
const LIMITE_MB = LIMITE_IMPORTACAO_BYTES / (1024 * 1024);

/** Campo de `SofaField` sem o `describedBy` (o `SofaInput` recebe o `aria-describedby` do controle). */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Soma o que a importação inseriu e atualizou, em todas as tabelas. */
function somas(relatorio: RelatorioImportacao): { inseridas: number; atualizadas: number } {
    let inseridas = 0;
    let atualizadas = 0;
    for (const linha of Object.values(relatorio.tabelas)) {
        inseridas += linha.inseridas;
        atualizadas += linha.atualizadas;
    }
    return { inseridas, atualizadas };
}

function numero(valor: number): string {
    return valor.toLocaleString("pt-BR");
}

/** Bloco de erro com a frase do operador e, quando houver, a lista de conflitos. */
function AvisoDeFalha({
    falha,
    aoTentarDeNovo,
}: {
    falha: FalhaDeBackup;
    aoTentarDeNovo?: () => void;
}) {
    // O `SofaBanner` de tom `danger` já anuncia como alerta (role="alert"); não se envolve de novo.
    return (
        <div>
            <SofaBanner tone="danger" title={falha.titulo}>
                <p style={{ margin: 0 }}>{falha.texto}</p>
                {falha.versoes ? (
                    <p style={{ margin: "6px 0 0" }}>
                        O arquivo é da versão {falha.versoes.arquivo}; esta instalação entende a versão{" "}
                        {falha.versoes.instalacao}.
                    </p>
                ) : null}
                {falha.conflitos && falha.conflitos.itens.length > 0 ? (
                    <>
                        <p style={{ margin: "6px 0 0" }}>Conflitos encontrados:</p>
                        <ul className="poph-operacao__conflitos" aria-label="Conflitos encontrados">
                            {falha.conflitos.itens.map((item) => (
                                <li key={item}>{item}</li>
                            ))}
                            {falha.conflitos.mais > 0 ? <li>e mais {numero(falha.conflitos.mais)}</li> : null}
                        </ul>
                    </>
                ) : null}
                {falha.podeTentarDeNovo && aoTentarDeNovo ? (
                    <div style={{ marginTop: 8 }}>
                        <SofaButton variant="secondary" onClick={aoTentarDeNovo}>
                            Tentar de novo
                        </SofaButton>
                    </div>
                ) : null}
            </SofaBanner>
        </div>
    );
}

/**
 * Recusa da importação que sobrescreveria linhas existentes. Nada foi gravado: a tela mostra o que
 * seria atualizado e só reenvia com `sobrescrever=true` quando o operador clica no botão de autorizar.
 * O botão não aparece para conflito de nome (`409 conflict`), que não tem como ser autorizado.
 */
function RecusaDeSobrescrita({
    recusa,
    importando,
    aoAutorizar,
    aoCancelar,
}: {
    recusa: RecusaSobrescrita;
    importando: boolean;
    aoAutorizar: () => void;
    aoCancelar: () => void;
}) {
    const tabelas = Object.entries(recusa.tabelas);
    // O `SofaBanner` já anuncia como alerta (role="alert"); não se envolve de novo.
    return (
        <SofaBanner tone="warn" title="Esta importação vai sobrescrever o que já existe">
            <p style={{ margin: 0 }}>
                {numero(recusa.linhas)} {recusa.linhas === 1 ? "linha desta organização seria atualizada" : "linhas desta organização seriam atualizadas"} com o conteúdo do arquivo.
                Nada foi gravado. Se é isso que você quer, autorize abaixo.
            </p>
            {tabelas.length > 0 ? (
                <table className="poph-operacao__tabela" aria-label="Linhas que seriam sobrescritas por tabela" style={{ marginTop: 8 }}>
                    <thead>
                        <tr>
                            <th scope="col">Tabela</th>
                            <th scope="col">Linhas</th>
                        </tr>
                    </thead>
                    <tbody>
                        {tabelas.map(([tabela, linhas]) => (
                            <tr key={tabela}>
                                <td>{nomeDaTabela(tabela)}</td>
                                <td>{numero(linhas)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            ) : null}
            {recusa.exemplos.length > 0 ? (
                <>
                    <p style={{ margin: "6px 0 0" }}>Exemplos do que seria atualizado:</p>
                    <ul className="poph-operacao__conflitos" aria-label="Exemplos do que seria sobrescrito">
                        {recusa.exemplos.map((item, indice) => (
                            <li key={`${indice}-${item}`}>{textoDeExemplo(item)}</li>
                        ))}
                    </ul>
                </>
            ) : null}
            <div style={{ marginTop: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
                <SofaButton variant="primary" loading={importando} disabled={importando} onClick={aoAutorizar}>
                    Sobrescrever e importar
                </SofaButton>
                <SofaButton variant="secondary" disabled={importando} onClick={aoCancelar}>
                    Cancelar
                </SofaButton>
            </div>
        </SofaBanner>
    );
}

/** Relatório da importação: o que entrou, segredos a digitar de novo e o estado do índice. */
function RelatorioDaImportacao({ relatorio }: { relatorio: RelatorioImportacao }) {
    const totais = somas(relatorio);
    const linhas = Object.entries(relatorio.tabelas).filter(
        ([, linha]) => linha.inseridas > 0 || linha.atualizadas > 0,
    );
    const indiceProntoAgora = relatorio.aviso === null;

    return (
        <div className="poph-operacao__relatorio" role="status" aria-live="polite">
            <p style={{ margin: "0 0 var(--pop-spacing-sm, 6px)" }}>
                {relatorio.org.criada
                    ? "Organização criada a partir do arquivo."
                    : "Conteúdo do arquivo gravado nesta organização."}{" "}
                {numero(relatorio.total)} linhas gravadas: {numero(totais.inseridas)} novas e{" "}
                {numero(totais.atualizadas)} atualizadas.
                {totais.inseridas === 0 && totais.atualizadas === 0
                    ? " Nada mudou: o conteúdo já estava aqui."
                    : ""}
            </p>

            {linhas.length > 0 ? (
                <table className="poph-operacao__tabela" aria-label="Linhas gravadas por tabela">
                    <thead>
                        <tr>
                            <th scope="col">Tabela</th>
                            <th scope="col">Novas</th>
                            <th scope="col">Atualizadas</th>
                        </tr>
                    </thead>
                    <tbody>
                        {linhas.map(([tabela, linha]) => (
                            <tr key={tabela}>
                                <td>{nomeDaTabela(tabela)}</td>
                                <td>{numero(linha.inseridas)}</td>
                                <td>{numero(linha.atualizadas)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            ) : null}

            {relatorio.segredos_a_redigitar > 0 ? (
                <div style={{ marginTop: "var(--pop-spacing-md, 8px)" }}>
                    <SofaBanner tone="warn" title="Segredos para digitar de novo">
                        {relatorio.segredos_a_redigitar === 1
                            ? "1 integração chegou sem o segredo, porque o arquivo não leva segredo."
                            : `${numero(relatorio.segredos_a_redigitar)} integrações chegaram sem o segredo, porque o arquivo não leva segredo.`}{" "}
                        Digite o segredo de novo na aba Integrações do agente antes de usar essas integrações.
                    </SofaBanner>
                </div>
            ) : null}

            <div style={{ marginTop: "var(--pop-spacing-md, 8px)" }}>
                {indiceProntoAgora ? (
                    <SofaBanner tone="success" title="Busca pronta">
                        Base de conhecimento: {numero(relatorio.indice.base_conhecimento.trechos)} trechos de{" "}
                        {numero(relatorio.indice.base_conhecimento.agentes)} agentes. Produtos:{" "}
                        {numero(relatorio.indice.produtos.trechos)} trechos. A busca já responde com o conteúdo
                        importado.
                    </SofaBanner>
                ) : (
                    <SofaBanner tone="warn" title="Importado, índice pendente">
                        O conteúdo entrou, mas a busca da base de conhecimento e dos produtos ainda não foi
                        montada: o modelo de embedding não está disponível nesta instância. Quando ele estiver
                        instalado, reindexe pela linha de comando (poph kb reindex e poph product reindex).
                    </SofaBanner>
                )}
            </div>
        </div>
    );
}

export type SecaoBackupProps = {
    /** Id da organização da tela (vem da URL). */
    org: string;
};

/**
 * A seção em si. Fica dentro da tela de operação, mas tem o próprio estado: a atualização de 15 s
 * da tela não mexe nela.
 */
export function ExportarImportarOrganizacao({ org }: SecaoBackupProps) {
    const idSecao = useId();
    const idEntrada = useId();

    const [comConversas, setComConversas] = useState(false);
    const [exportando, setExportando] = useState(false);
    const [avisoExportacao, setAvisoExportacao] = useState<string | null>(null);
    const [falhaExportacao, setFalhaExportacao] = useState<FalhaDeBackup | null>(null);

    const [arquivo, setArquivo] = useState<File | null>(null);
    const [confirmando, setConfirmando] = useState(false);
    const [importando, setImportando] = useState(false);
    const [relatorio, setRelatorio] = useState<RelatorioImportacao | null>(null);
    const [falhaImportacao, setFalhaImportacao] = useState<FalhaDeBackup | null>(null);
    const [recusa, setRecusa] = useState<RecusaSobrescrita | null>(null);
    const [arquivoRecusado, setArquivoRecusado] = useState(false);
    /** Trocado após importar, para o campo de arquivo começar vazio de novo. */
    const [versaoEntrada, setVersaoEntrada] = useState(0);

    async function exportar() {
        setExportando(true);
        setAvisoExportacao(null);
        setFalhaExportacao(null);
        try {
            const exportado = await exportarOrganizacao(org, comConversas);
            baixarArquivo(exportado);
            setAvisoExportacao(`Arquivo "${exportado.nome}" baixado.`);
        } catch (erro) {
            setFalhaExportacao(falhaDeBackup(erro, "exportar"));
        } finally {
            setExportando(false);
        }
    }

    function escolherArquivo(escolhido: File | null) {
        setRelatorio(null);
        setFalhaImportacao(null);
        setRecusa(null);
        if (!escolhido) {
            setArquivo(null);
            setArquivoRecusado(false);
            return;
        }
        // Arquivo acima do teto nem vai para a fila: a tela mostra o tamanho e recusa.
        const recusado = passaDoLimite(escolhido.size);
        setArquivoRecusado(recusado);
        setArquivo(escolhido);
    }

    /**
     * Envia o arquivo. Sem `sobrescrever` (o padrão), o servidor recusa se a importação atualizaria
     * linhas existentes e a tela mostra a recusa com o botão de autorizar. Com `true`, só vai quando o
     * operador clicou em "Sobrescrever e importar".
     */
    async function importar(sobrescrever: boolean) {
        if (!arquivo) return;
        setImportando(true);
        setRelatorio(null);
        setFalhaImportacao(null);
        setRecusa(null);
        try {
            const texto = await lerTextoDoArquivo(arquivo);
            const resultado = await importarOrganizacao(org, texto, sobrescrever);
            setRelatorio(resultado);
            setArquivo(null);
            setArquivoRecusado(false);
            setVersaoEntrada((versao) => versao + 1);
        } catch (erro) {
            const recusaDoServidor = recusaDeSobrescrita(erro);
            if (recusaDoServidor) {
                setRecusa(recusaDoServidor);
            } else {
                setFalhaImportacao(falhaDeBackup(erro, "importar"));
            }
        } finally {
            setImportando(false);
            setConfirmando(false);
        }
    }

    const podeImportar = arquivo !== null && !arquivoRecusado && !importando;

    return (
        <section className="poph-secao" aria-labelledby={idSecao}>
            <h2 id={idSecao} className="poph-secao__titulo">Exportar e importar a organização</h2>
            <p className="poph-operacao__ajuda">
                Exportar gera um arquivo com a configuração, o catálogo, a base de conhecimento e a biblioteca
                desta organização. Importar grava esse arquivo numa organização, sem apagar o que já existe nela.
                Segredos de integração não vão no arquivo: depois de importar, é preciso digitá-los de novo.
            </p>

            <h3 className="poph-operacao__subtitulo">Exportar</h3>
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <SofaCheckbox
                    label="Incluir conversas"
                    description="Leva mensagens, turnos, anexos e as filas de entrada e saída. Sem isso, o arquivo traz só a configuração, o catálogo, a base e a biblioteca."
                    checked={comConversas}
                    disabled={exportando}
                    onChange={(evento) => setComConversas(evento.target.checked)}
                />
                <div>
                    <SofaButton
                        variant="secondary"
                        loading={exportando}
                        disabled={exportando}
                        onClick={() => void exportar()}
                    >
                        Baixar arquivo da organização
                    </SofaButton>
                </div>
                <div aria-live="polite">
                    {exportando ? (
                        <p className="poph-operacao__carregando" role="status">
                            Gerando o arquivo da organização… isso pode demorar em organizações com biblioteca grande.
                        </p>
                    ) : null}
                    {avisoExportacao && !exportando ? (
                        <p className="poph-operacao__carregando" role="status">
                            {avisoExportacao}
                        </p>
                    ) : null}
                    {falhaExportacao ? (
                        <AvisoDeFalha
                            falha={falhaExportacao}
                            aoTentarDeNovo={() => void exportar()}
                        />
                    ) : null}
                </div>
            </div>

            <h3 className="poph-operacao__subtitulo">Importar</h3>
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <p className="poph-operacao__ajuda" style={{ margin: 0 }}>
                    Importar não apaga o que já existe nesta organização. Se algum recurso do arquivo já existir
                    aqui com o mesmo nome, a importação para antes de gravar e nada é alterado.
                </p>
                <SofaField
                    label="Arquivo de backup"
                    hint={`Arquivo .json gerado por esta tela ou pelo comando de exportar. Até ${tamanhoEmMb(LIMITE_IMPORTACAO_BYTES)}.`}
                    error={arquivoRecusado && arquivo ? `O arquivo tem ${tamanhoEmMb(arquivo.size)}, acima do limite de ${LIMITE_MB} MB. Nada foi enviado.` : undefined}
                    htmlFor={idEntrada}
                >
                    {(controle) => (
                        <SofaInput
                            key={versaoEntrada}
                            {...semDescribedBy(controle)}
                            invalid={arquivoRecusado}
                            type="file"
                            accept=".json,application/json"
                            disabled={importando}
                            onChange={(evento) => escolherArquivo(evento.target.files?.[0] ?? null)}
                        />
                    )}
                </SofaField>
                {arquivo && !arquivoRecusado ? (
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        Escolhido: {arquivo.name} · {tamanhoEmMb(arquivo.size)} (dentro do limite)
                    </p>
                ) : null}
                <div>
                    <SofaButton
                        variant="primary"
                        disabled={!podeImportar}
                        loading={importando}
                        onClick={() => setConfirmando(true)}
                    >
                        Importar arquivo nesta organização…
                    </SofaButton>
                </div>

                <div aria-live="polite">
                    {importando ? (
                        <p className="poph-operacao__carregando" role="status">
                            Importando o arquivo… não feche a tela até aparecer o resultado.
                        </p>
                    ) : null}
                    {falhaImportacao ? (
                        <AvisoDeFalha
                            falha={falhaImportacao}
                            aoTentarDeNovo={falhaImportacao.podeTentarDeNovo && arquivo ? () => setConfirmando(true) : undefined}
                        />
                    ) : null}
                    {recusa ? (
                        <RecusaDeSobrescrita
                            recusa={recusa}
                            importando={importando}
                            aoAutorizar={() => void importar(true)}
                            aoCancelar={() => setRecusa(null)}
                        />
                    ) : null}
                    {relatorio ? <RelatorioDaImportacao relatorio={relatorio} /> : null}
                </div>
            </div>

            <SofaConfirmDialog
                open={confirmando}
                onOpenChange={(aberto) => {
                    if (!aberto && !importando) setConfirmando(false);
                }}
                title="Importar este arquivo nesta organização?"
                description={
                    arquivo
                        ? `O arquivo ${arquivo.name} (${tamanhoEmMb(arquivo.size)}) será gravado nesta organização. A importação não apaga o que já existe aqui. Se algum recurso tiver o mesmo nome, nada é gravado. Segredos de integração precisam ser digitados de novo.`
                        : "Nenhum arquivo escolhido."
                }
                confirmLabel="Importar agora"
                cancelLabel="Cancelar"
                closeLabel="Fechar confirmação de importação"
                onConfirm={() => void importar(false)}
                loading={importando}
            />
        </section>
    );
}
