# Exemplo 1 — o mais simples possível: duas etapas e dois fins, sem decisão.
# Serve para conferir a forma mínima de um fluxo válido.
fluxo(
    id = "agendar_visita",
    nome = "Agendar visita",
    descricao = "Marca uma visita técnica no endereço do cliente.",
    inicio = "coleta",
    gatilho = gatilho(
        quando = "O cliente quer marcar, remarcar ou cancelar uma visita técnica.",
        nao_quando = "Só quer saber preço ou prazo, sem pedir visita.",
        exemplos = ["queria agendar uma visita", "pode vir aqui amanhã?"],
    ),
)

etapa(
    id = "coleta",
    nome = "Coletar dados",
    instrucao = "Pergunte o endereço e o período preferido. Uma pergunta por vez.",
    campos = [
        campo("endereco", "Endereço completo", tipo = "texto", obrigatorio = True),
        campo("periodo", "Período", tipo = "opcao", opcoes = ["manhã", "tarde"], obrigatorio = True),
    ],
    acoes = [
        acao("confirmar", "Dados completos", exige = ["endereco", "periodo"], vai_para = "confirmacao"),
        acao("desistir", "Cliente desistiu", vai_para = fim("desistencia", resultado = "Cliente desistiu")),
    ],
)

etapa(
    id = "confirmacao",
    nome = "Confirmar visita",
    instrucao = "Repita o endereço e o período combinados e peça a confirmação do cliente.",
    acoes = [
        acao("ok", "Visita combinada", vai_para = fim("concluida", resultado = "Visita agendada")),
    ],
)
