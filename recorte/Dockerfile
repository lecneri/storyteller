# Recorte de fundo (rembg) para as imagens dos personagens. Só na rede interna: POST com a imagem → PNG sem fundo.
FROM python:3.12-slim
RUN pip install --no-cache-dir "rembg[cpu]==2.0.85" \
 && useradd -m recorte
ENV U2NET_HOME=/modelos MODELO=isnet-general-use
RUN mkdir /modelos && chown recorte /modelos
USER recorte
# Baixa o modelo no build (fica na imagem; nada é baixado em uso).
RUN python -c "import os; from rembg import new_session; new_session(os.environ['MODELO'])"
COPY --chown=recorte servidor.py /app/servidor.py
EXPOSE 7000
CMD ["python", "/app/servidor.py"]
