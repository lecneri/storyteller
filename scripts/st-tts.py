#!/usr/bin/env python3
"""TTS do Storyteller (edge-tts). Lê JSON no stdin: {texto, voz, tom, velocidade, saida}.
Grava o MP3 em `saida` e imprime JSON com os tempos de cada palavra (para o destaque karaokê)."""
import asyncio
import json
import sys

import edge_tts


async def main() -> None:
    p = json.load(sys.stdin)
    com = edge_tts.Communicate(
        p["texto"],
        p["voz"],
        rate=p.get("velocidade") or "+0%",
        pitch=p.get("tom") or "+0Hz",
        boundary="WordBoundary",
    )
    palavras = []
    with open(p["saida"], "wb") as f:
        async for parte in com.stream():
            if parte["type"] == "audio":
                f.write(parte["data"])
            elif parte["type"] == "WordBoundary":
                # offset/duração vêm em unidades de 100 ns
                palavras.append([round(parte["offset"] / 1e7, 3), round(parte["duration"] / 1e7, 3), parte["text"]])
    print(json.dumps({"palavras": palavras}, ensure_ascii=False))


asyncio.run(main())
