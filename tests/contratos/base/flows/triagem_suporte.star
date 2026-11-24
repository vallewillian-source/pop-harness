# Exemplo 2 — decisão nas duas formas: combinador declarativo e função `def`.
# A etapa "classificar" não é uma etapa: é uma decisão. Ela lê os dados já
# registrados e segue para uma etapa ou para um fim.
fluxo(
    id = "triagem_suporte",
    nome = "Triagem de suporte",
    descricao = "Descobre o problema, classifica a urgência e encaminha o atendimento.",
    inicio = "relato",
    gatilho = gatilho(
        quando = "O cliente relata um problema técnico e precisa de ajuda para resolver.",
        nao_quando = "O cliente quer comprar, agendar visita ou falar de cobrança.",
        exemplos = ["meu sistema não abre", "deu erro ao emitir a nota"],
    ),
)

etapa(
    id = "relato",
    nome = "Ouvir o relato",
    instrucao = "Peça o que aconteceu e se o trabalho está parado agora. Uma pergunta por vez.",
    campos = [
        campo("resumo", "O que aconteceu", tipo = "texto", obrigatorio = True),
        campo("afeta_agora", "Está parado agora?", tipo = "opcao", opcoes = ["sim", "nao"], obrigatorio = True),
    ],
    acoes = [
        acao("registrado", "Relato registrado", exige = ["resumo", "afeta_agora"], vai_para = "classificar"),
        acao("nao_e_suporte", "Não é suporte", vai_para = fim("fora_de_escopo", resultado = "Fora do suporte")),
    ],
)

decisao(
    id = "classificar",
    se = e(
        igual("relato.afeta_agora", "sim"),
        funcao("problema_grave"),
    ),
    entao = "urgente",
    senao = "normal",
)

etapa(
    id = "urgente",
    nome = "Atendimento urgente",
    instrucao = "Avise que o time foi acionado e que respondemos em até 30 minutos.",
    acoes = [
        acao("encaminhado", "Encaminhado ao time", vai_para = fim("concluido_urgente", resultado = "Urgente")),
    ],
)

etapa(
    id = "normal",
    nome = "Fila normal",
    instrucao = "Informe o prazo de um dia útil e registre o contato do cliente.",
    acoes = [
        acao("encaminhado", "Encaminhado ao time", vai_para = fim("concluido_normal", resultado = "Normal")),
    ],
)

def problema_grave(dados):
    # Regra avançada: recebe os dados do fluxo e devolve True ou False.
    # Só lê o que está na ficha; não tem relógio, rede, arquivo nem aleatório.
    resumo = dados.get("relato", {}).get("resumo", "").lower()
    for palavra in ["parado", "não abre", "nao abre", "perdi", "urgente"]:
        if palavra in resumo:
            return True
    return len(resumo) > 200
