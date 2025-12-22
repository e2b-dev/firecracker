-include .env

.PHONY: build
build:
	./scripts/build.sh

.PHONY: upload
upload:
	./scripts/upload.sh $(GCP_PROJECT_ID)

.PHONY: build-and-upload
build-and-upload: build upload
