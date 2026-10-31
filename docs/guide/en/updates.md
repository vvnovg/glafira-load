# Updates

This chapter covers updating the app, the [[ui:sidebar.about]] screen and supporting the project.

## Updates {#updates}

At startup, the app checks once whether a new version is available. If it is, a notice with the version number appears above the tabs. The [[ui:about.notice.more]] link opens the [[ui:sidebar.about]] screen. If the check fails, for example without internet access, nothing is shown.

To check manually:

1. Open [[ui:sidebar.about]] at the bottom of the left panel.
2. In the [[ui:about.updates.title]] section, click [[ui:about.updates.check]].
3. If a new version is available, the screen shows its number and the description of the changes. Click [[ui:about.updates.install]].

The app downloads the update, verifies its signature, installs it and restarts. If a network failure interrupts the download, it is retried automatically, up to three attempts in total. If all three attempts fail, the app reports: "[[ui:errors.updates.downloadInterrupted]]". If the signature fails verification, the app reports: "[[ui:errors.updates.badSignature]]".

Installing may ask for an administrator password, because the app is installed for all users of the computer: by the `.pkg` installer on macOS, the `.msi` installer on Windows, or a `.deb` or `.rpm` package on Linux. Users without administrator rights cannot update such an installation.

Your projects stay in their own folders; the settings and other data of the app are kept after the update.

Updates are downloaded from the releases page on GitHub. If the check ends with "[[ui:errors.updates.checkFailed]]", check your network or proxy. "[[ui:errors.updates.notConfigured]]" means that the build was made without the update key, for example a development or test build. In that case, download new versions from the releases page: https://github.com/vvnovg/glafira-load-releases/releases

Your organization may turn off the update check with a policy or point it to an internal mirror. When the check is off, "[[ui:errors.updates.managed]]" appears in the [[ui:about.updates.title]] section after you click [[ui:about.updates.check]]. Like "[[ui:errors.updates.notConfigured]]", this message is shown only after the click: the check at startup shows nothing in such cases.

## About {#about}

Open [[ui:sidebar.about]] at the bottom of the left panel. The screen shows a short description of the app and these lines:

- [[ui:about.version]] — the version number, and [[ui:about.build]] — the identifier of the source code the app was built from. Give both values when you contact support.
- [[ui:about.system]] — the operating system and processor architecture.
- [[ui:about.license]] — Apache 2.0; the link opens the license text in the browser.
- [[ui:about.policy]] — "[[ui:about.policySet]]" or "[[ui:about.policyNone]]". Policy errors are listed below.
- [[ui:about.dataDir]] — the folder with the settings of the app.
- [[ui:about.logDir]] — the folder with log files.

The [[ui:about.updates.title]] section is on the same screen.

## Support the project {#support}

Glafira for Load Testing is free, and its source code is open under the Apache License 2.0. The project lives on donations. The [[ui:support.title]] item at the bottom of the left panel and on the [[ui:sidebar.about]] screen opens the support page in the browser: https://github.com/vvnovg/glafira-load-releases/blob/main/SUPPORT.md
