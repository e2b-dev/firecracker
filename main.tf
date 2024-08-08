terraform {
  required_version = ">= 1.5.0, < 1.6.0"
  backend "gcs" {
    prefix = "terraform/firecracker/state"
  }
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "5.25.0"
    }
    github = {
      source  = "integrations/github"
      version = "5.42.0"
    }
  }
}

data "google_client_config" "default" {}

provider "google" {
  project = var.gcp_project_id
  region  = var.gcp_region
  zone    = var.gcp_zone
}



module "github_tf" {
  source = "./github-tf"

  gcp_project_id = var.gcp_project_id
  gcp_region     = var.gcp_region
  gcp_zone       = var.gcp_zone

  github_organization = var.github_organization
  github_repository   = var.github_repository

  fc_versions_bucket = "${var.gcp_project_id}-fc-versions"

  prefix = var.prefix
}
