# A WebSocket echo origin that negotiates permessage-deflate, which Autobahn's
# own echoserver does not. See server.py.
FROM python:3.12-slim

RUN pip install --no-cache-dir "websockets>=12,<16"

COPY server.py /app/server.py
WORKDIR /app

EXPOSE 9003
ENTRYPOINT ["python3", "/app/server.py", "0.0.0.0", "9003"]
