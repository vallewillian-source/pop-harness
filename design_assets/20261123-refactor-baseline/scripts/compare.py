#!/usr/bin/env python3
import sys
import os
import glob
from PIL import Image, ImageChops, ImageDraw

# Tolerância por canal (0-255) para absorver diferenças imperceptíveis de sub-pixel antialiasing
COLOR_TOLERANCE = 12

# Limite máximo de pixels com divergência aceitos como ruído fora de regiões voláteis (~0.001% de 1.296.000 px)
MAX_UNEXPECTED_PIXELS = 15

# Regiões voláteis documentadas por tela: (x1, y1, x2, y2)
# Qualquer alteração fora destas caixas reprova a comparação.
VOLATILE_REGIONS = {
    # 01-login: 100% determinística (sem regiões voláteis)
    "01-login.png": [],

    # 02-orgs-lista:
    # 1. Topbar: chip de estado e memória do motor (Engine: 200 OK · [N] MB)
    # 2. Orgs: subtítulo de telemetria (1 nó conectado · Uptime ... · RAM ...)
    "02-orgs-lista.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
        {"nome": "Orgs: telemetria (uptime/RAM)", "box": (230, 72, 500, 96)},
    ],

    # 03-orgs-inspetor:
    # 1. Topbar: chip de estado e memória do motor
    # 2. Orgs: subtítulo de telemetria
    "03-orgs-inspetor.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
        {"nome": "Orgs: telemetria (uptime/RAM)", "box": (230, 72, 500, 96)},
    ],

    # 04-agente-instrucoes:
    # 1. Topbar: chip de estado e memória do motor
    "04-agente-instrucoes.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
    ],

    # 05-agente-base-conhecimento:
    # 1. Topbar: chip de estado e memória do motor
    "05-agente-base-conhecimento.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
    ],

    # 06-chat-teste:
    # 1. Topbar: chip de estado e memória do motor
    "06-chat-teste.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
    ],

    # 07-operacao:
    # 1. Topbar: chip de estado e memória do motor
    # 2. Timestamp de atualização dinâmica da página
    # 3. Tempo no ar do processo
    # 4. Memória usada pelo processo
    "07-operacao.png": [
        {"nome": "Topbar: chip de memória/engine", "box": (1050, 10, 1264, 48)},
        {"nome": "Operação: timestamp de atualização", "box": (1170, 100, 1304, 136)},
        {"nome": "Operação: tempo no ar", "box": (730, 298, 880, 318)},
        {"nome": "Operação: memória usada pelo processo", "box": (730, 663, 880, 683)},
    ],
}

def compare_dirs(dir1, dir2):
    print("==================================================")
    print("Verificando estabilidade entre capturas:")
    print(f"Base:    {dir1}")
    print(f"Segunda: {dir2}")
    print("==================================================")

    if not os.path.isdir(dir1):
        print(f"ERRO: Diretório base não encontrado: {dir1}")
        sys.exit(1)
    if not os.path.isdir(dir2):
        print(f"ERRO: Diretório de comparação não encontrado: {dir2}")
        sys.exit(1)

    files1 = sorted(glob.glob(os.path.join(dir1, "*.png")))
    if not files1:
        print(f"ERRO: Nenhuma imagem encontrada em {dir1}")
        sys.exit(1)

    all_stable = True
    for p1 in files1:
        fname = os.path.basename(p1)
        p2 = os.path.join(dir2, fname)
        if not os.path.exists(p2):
            print(f"[FAIL] {fname}: arquivo ausente na segunda captura ({p2})")
            all_stable = False
            continue

        try:
            im1 = Image.open(p1).convert("RGB")
            im2 = Image.open(p2).convert("RGB")
        except Exception as e:
            print(f"[FAIL] {fname}: erro ao carregar imagens: {e}")
            all_stable = False
            continue

        if im1.size != im2.size:
            print(f"[FAIL] {fname}: dimensões divergentes ({im1.size} vs {im2.size})")
            all_stable = False
            continue

        diff = ImageChops.difference(im1, im2)
        r, g, b = diff.split()[:3]
        max_diff = ImageChops.lighter(ImageChops.lighter(r, g), b)
        thresh = max_diff.point(lambda p: 255 if p > COLOR_TOLERANCE else 0)

        bbox = thresh.getbbox()
        if bbox is None:
            print(f"[OK] {fname}: 100% IDÊNTICA (zero diferenças)")
            continue

        allowed_regions = VOLATILE_REGIONS.get(fname, [])

        # Isolar divergências inesperadas fora das regiões voláteis
        unexpected_thresh = thresh.copy()
        draw_unexp = ImageDraw.Draw(unexpected_thresh)
        for reg in allowed_regions:
            draw_unexp.rectangle(reg["box"], fill=0)

        unexpected_bbox = unexpected_thresh.getbbox()
        unexpected_pixels = 0
        if unexpected_bbox is not None:
            crop_unexp = unexpected_thresh.crop(unexpected_bbox)
            unexpected_pixels = sum(1 for p in crop_unexp.tobytes() if p > 0)

        if unexpected_pixels > MAX_UNEXPECTED_PIXELS:
            x1, y1, x2, y2 = unexpected_bbox
            w = x2 - x1
            h = y2 - y1
            print(f"[FAIL] {fname}: {unexpected_pixels} pixels alterados fora das regiões voláteis")
            print(f"       Caixa delimitadora: [x={x1}, y={y1}, w={w}, h={h}]")
            all_stable = False
        else:
            # Avaliar se houve variação em regiões voláteis
            vmask = Image.new("L", thresh.size, 0)
            vdraw = ImageDraw.Draw(vmask)
            for reg in allowed_regions:
                vdraw.rectangle(reg["box"], fill=255)
            volatile_diff = ImageChops.darker(thresh, vmask)
            v_bbox = volatile_diff.getbbox()
            if v_bbox is not None:
                v_crop = volatile_diff.crop(v_bbox)
                v_pixels = sum(1 for p in v_crop.tobytes() if p > 0)
                vx1, vy1, vx2, vy2 = v_bbox
                vw = vx2 - vx1
                vh = vy2 - vy1
                reg_names = [reg["nome"] for reg in allowed_regions]
                print(f"[VOLÁTIL] {fname}: {v_pixels} pixels em campos dinâmicos documentados ({', '.join(reg_names)})")
                print(f"          Caixa: [x={vx1}, y={vy1}, w={vw}, h={vh}]")
            else:
                print(f"[OK] {fname}: idêntica (diferenças residuais dentro da tolerância de ruído)")

    print("==================================================")
    if not all_stable:
        print("FALHA: Divergências visuais detectadas fora do esperado.")
        print("==================================================")
        sys.exit(1)
    else:
        print("Verificação de estabilidade concluída com sucesso!")
        print("==================================================")
        sys.exit(0)

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Uso: compare.py <dir1> <dir2>")
        sys.exit(1)
    compare_dirs(sys.argv[1], sys.argv[2])
