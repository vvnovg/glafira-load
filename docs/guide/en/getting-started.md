# Getting started

Glafira for Load Testing is a desktop tool for preparing and maintaining load tests. This version (stage L1) builds the
registry of operations from the contracts of your services, exports the production-load template to Excel, imports the
filled template and computes the load profile. The CLI runner, suites, mocks and reports come in the next stages.

Everything Glafira produces is kept in the project folder as plain YAML files that you commit to git next to your
tests. The app reads the clones of your services but never changes them. It sends nothing over the network except the
update check, and your organization can turn that off with a policy.

## Installation {#install}

Installers for macOS (`.dmg`), Windows (`.msi`) and Linux (`.deb`, `.rpm`, `.AppImage`) are published at
https://github.com/vvnovg/glafira-load-releases/releases. Your organization can configure the app with a policy:
then some settings are set by the administrator.

## First project {#first-project}

A load project is a folder in git: the manifest `glafira-load.yaml`, the operation registry, the production-load
figures and the load profile. The services under test are local git clones somewhere on your disk.

1. Click [[ui:sidebar.newProject]] in the left panel.
2. Enter the [[ui:project.name]], click [[ui:project.chooseFolder]] and pick an empty folder inside your git
   repository, then click [[ui:project.create]]. The project appears in the left panel under [[ui:sidebar.projects]].
3. On the project tab, click [[ui:project.addService]], enter the [[ui:project.serviceId]] (lowercase letters,
   digits and hyphens) and choose the folder of the service clone. Click [[ui:common.save]].
4. Open the service in the left panel under [[ui:sidebar.services]]. Glafira lists the contracts it found in the
   clone. Click [[ui:service.scan]].
5. Open the [[ui:sidebar.registry]]: it lists the operations of the service with their request and response schemas.

An existing project is opened with [[ui:sidebar.openProject]]. Next steps: export the production-load template
([Production load](prod-load.md)) and compute the load profile ([Load profile](profile.md)). More about the project
folder in [Project and services](project.md).

## Interface language {#language}

Open [[ui:sidebar.settings]] and choose the [[ui:settings.language.label]]. By default it follows the system. If the
language is set by your organization, it cannot be changed.

## Help {#help}

F1 on any screen opens its chapter of this guide. The guide works offline; search is in the field above the contents.
Many fields also have a "?" hint next to them.
