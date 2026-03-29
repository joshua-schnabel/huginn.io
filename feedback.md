Hej :)


ich dumpe hier einfach meine Opinionatede Meinung hin. KI hab ich jetzt nicht gefragt, weil ich davon ausgehe, dass du das schon selbst gemacht hast.

Ich hab die Punkte der Einfachheit (falls du Rückfragen hast) nummeriert, die Nummern haben aber nichts zu sagen. 

Vorab möchte ich aber noch erwähnen, dass ich das alles in allem okay finde. Technologieauswahl - Doku - Code ist mir jetzt nichts herausragendes aufgefallen. Dennoch biete ich dir hier an, meine Meinung einzuverleiben:


A. Readme

1. In der Readme gehen die ersten beiden Links (CI und SAST) nicht. 

2. Wobei ich SAST noch nie gehört habe. Habs jetzt gegoogelt, aber würde ich nicht so (unkommentiert) im Readme lassen

3. Mit "protocols" meinst du wahrscheinlich die Probes, oder? Würde ich besser erklären

4. Zur Config: Bist du dir sicher, dass du yaml willst? Ich hab schon gesehen, dass du in dem Cargo.toml serde_yml verwendet - generell erstmal keine abwegige Idee. Aber das Crate ist verrufen, und war bis vor kurzem mehrere Jahre un-maintained. Der neue Maintainer macht das auch nur noch pro-forma. YAML ist halt mega kompliziert, und serde_yml hat nicht mehr den Anspruch, das korrekt zu machen, sondern nur "irgendwie". Das crate hat bei Rust einen wirklich schlechten Ruf. (im Gegensatz zu den anderen Serde_*, die hervorragend sind). Rust ist generell eher Fan von TOML. 

B. Code


5. Ist jetzt nichts Rust-Spezifisches, aber generell ist mir ein Ungleichgewicht aufgefallen: Zum einen ist die Makro-Architektur sehr ausgearbeitet, es bestehen 5 Crates, die fast alle leer sind. Das finde ich persönlich unübersichtlich und wartungsaufwändig, v.a. weil ja nichts wirklich in den Crates drin ist. Ich weiß, JavaScript modularisiert quasi jede Zeile, aber Javascript ist das beste Beispiel, dass das auch keine gute Idee ist. 

6. Zum anderen ist die Mikro-Architektur für meinen Geschmack etwas zu dürftig. Deine Main-Methode enthält für mich viel zu viel Detail, was für mich den Ablauf des Programms schwer erkennbar macht. Gerade da bietet dir Rust die perekte Umgebung für "Syntactic Sugar" - denn der Compiler in-lined dir sowieso alle Methoden und alle Typen, da macht das im Gegensatz zu Kotlin oder Typescript wirklich garnichts aus, ob der Code schön oder optimiert aussieht. 

Generell schreibe ich ja gern auch mit der Actix Runtime (basiert ja ebenfalls auf Tokio), und die bieten zum Beispiel Typen-Aliase für alles mögliche an, was den Code schöner macht. Arc<&BlaBla> wird dann zu Data<BlaBla>. Nicht dass ich in dem Fall Actix besser finden würde (dafür kenne ich jetzt den Use-Case nicht ganz so gut), aber wenn du das händisch machen möchtest würde ich dir da mehr Extract-Method, Type-Alias und andere Spezifika des Clean Codes ans Herz legen. (zum beispiel auch das Shutdown-Dingsi kapseln, die Main-Method mal aufräumen).

Generell bin ich da etwas Fan vom Rust-Type-System, was dir deutlich schöneren und eleganteren Code ermöglicht als Typescript/Kotlin/Python.

7. In den Test-Foldern finde ich viel duplizierten Code. Könnte man aufräumen. Gerade start_server oder free_port hast du mindestens ein paar Mal im Code.

8. Das ist mir auch schon bei den Probes aufgefallen. Gerade das Time-Out-Verhalten lässt sich doch bestimmt extrahieren, oder?

9. Auch sowas wie Ok(Ok(resp)) hätte ich zum beispiel auf ein Enum gemapped. 

10. Jetzt, wo ich sehe, dass die Unittests in den Probes ja auch Server starten, habe ich die Unterscheidung zwischen "Test im gleichen Modul (Datei)" und "Test im test-Folder" noch nicht ganz verstanden

11. Sicher, dass du den Scheduler in das "probes" crate haben willst? Und nicht im main?

12. Tokyo ist dev-dependency und normale dependency gleichzeitig?


C. Sonstiges

13. Mit der Contributing.md stiftest du bei mir mehr Verwirrung als Klarheit. Eine einfache Erklärung wie ich einen PR machen kann wäre mir persönlich lieber

14. Dein Beschreibungs-Text in github ist deutsch? Also das Rechts-Oben?

15. Kein Release bisher, ebenso keine github-pages. GPT 5.2 generiert dir ne nette github-pages Seite, die deutlich besser als dein "docs"-Folder ist.


