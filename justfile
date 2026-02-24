set shell := ["bash", "-lc"]

dev:
  docker compose up --build

stop:
  docker compose down

logs:
  docker compose logs -f

restart:
  docker compose down
  docker compose up --build
