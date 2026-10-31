# Getting started

Glafira for Load Testing is a desktop tool for preparing and maintaining load tests. This version (stage L1) builds the
registry of operations from the contracts of your services, exports the production-load template to Excel, imports the
filled template and computes the load profile. The CLI runner, suites, mocks and reports come in the next stages.
Glafira keeps everything in the project folder, which is a git repository, and sends nothing over the network.

## Installation {#install}

Installers for macOS (`.dmg`), Windows (`.msi`) and Linux (`.deb`, `.rpm`, `.AppImage`) are published at
https://github.com/vvnovg/glafira-load-releases/releases. Your organization can configure the app with a policy:
then some settings are set by the administrator.

## First project {#first-project}

A load project is a folder in git: the manifest `glafira-load.yaml`, the operation registry, the production-load
figures and the load profile.

1. Click [[ui:sidebar.newProject]] in the left panel to create a project, or [[ui:sidebar.openProject]] to open an
   existing folder.
2. The project opens in its own tab, and its services, registry, production load and load profile appear in the left
   panel. More in [Project and services](project.md).

## Interface language {#language}

Open [[ui:sidebar.settings]] and choose the [[ui:settings.language.label]]. By default it follows the system. If the
language is set by your organization, it cannot be changed.

## Help {#help}

F1 on any screen opens its chapter. The guide works offline; search is in the field above the contents.
