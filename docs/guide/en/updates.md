# Updates

This chapter covers updating the app, the [[ui:sidebar.about]] screen and supporting the project.

## Updates {#updates}

Updates from inside the app come in a later version: the installers of this version are not signed for automatic
updates yet. Until then, download new versions from the releases page and install them over the old one:
https://github.com/vvnovg/glafira-load-releases/releases

Your projects stay in their own folders, and the settings of the app are kept when you install a new version.

The [[ui:about.updates.title]] section of the [[ui:sidebar.about]] screen is already there. In this version,
[[ui:about.updates.check]] answers "[[ui:errors.updates.notConfigured]]", and the app makes no request to the update
server. When updates are turned on, the app will check once at startup and show a notice above the tabs if a new
version is available.

Your organization can turn off the update check with the `updatesEnabled` policy or point it to an internal mirror with
`updateUrl`. When the check is off, [[ui:about.updates.check]] answers "[[ui:errors.updates.managed]]" and the app never
contacts the update server.

## About {#about}

Open [[ui:sidebar.about]] at the bottom of the left panel. The screen shows a short description of the app and these lines:

- [[ui:about.version]] — the version number, and [[ui:about.build]] — the identifier of the source code the app was built from. Give both values when you contact support.
- [[ui:about.system]] — the operating system and processor architecture.
- [[ui:about.license]] — Apache 2.0; the link opens the license text in the browser.
- [[ui:about.policy]] — "[[ui:about.policySet]]" or "[[ui:about.policyNone]]". Policy errors are listed below.
- [[ui:about.dataDir]] — the folder with the settings of the app.
- [[ui:about.logDir]] — the folder with log files.

The [[ui:about.agent.title]] section shows the command that downloads the load agent for the operating system and
processor of this machine ([Installing an agent](agents.md#install)). The [[ui:about.updates.title]] section is on the
same screen.

## Support the project {#support}

Glafira for Load Testing is free, and its source code is open under the Apache License 2.0. The project lives on donations. The [[ui:support.title]] item at the bottom of the left panel and on the [[ui:sidebar.about]] screen opens the support page in the browser: https://github.com/vvnovg/glafira-load-releases/blob/main/SUPPORT.md
