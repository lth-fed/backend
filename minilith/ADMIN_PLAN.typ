Write helper functions for all access checks, to keep code as minimal as possible.
A bunch of checks are already available at `./src/group/admin.rs`.
Check off items in this document as they are done. Continue until it's done. Please stop and ask for any ambiguities instead of assuming what I mean.
Many of the GET handlers are already implemented and only need to be tweaked for them to fulfil the requirements of this document.

= Aktiviteter & biljetter

alla dessa behöver man ha direkta adminships för. En direkt adminship i någon
av activity_hosts räcker.
alla adminkonton behöver också ha ett ID som börjar på `email:`, alltså kan man bara logga in med mail på sitt adminkonto. Det kan t.ex. kollas i databasen och när man insertar nya adminships

- [x] vanlig användare & admin: ladda alla grupper (alla som finns med i ens träd, alltså gå till root-noden och få alla underträd)
- [x] put aktivitet (activity_hosts)
- [x] get aktivitet med detaljer
- [x] put biljetttyp (med allowed_groups, addons & options) (följ get requestsen som redan finns i `./src/ticket.rs`, `./src/activities.rs`)
- [x] biljettyp get detaljer & om någon köpt den
- [x] put notifikation för biljettyp
- [x] get notifikation för biljettyp
- [x] access checks
- [x] lista köpta biljetter för aktivitet

- [x] ändra personinställningar för filtrering

= Grupper

- [x] lista med vilka grupper jag kan förfråga att gå med i (inkl. om jag redan förfrågat)
- [x] förfråga att gå med i grupp
- [x] admin: lista med vem som förfrågat att gå med grupp
- [x] admin: lista med vilka grupper som kan förfråga att gå med i en grupp
- [x] admin: acceptera förfrågan
- [x] admin: ändra grupp som man har direkt adminship i
- [x] create subgroup (direct adminship i föräldern)
- [x] hide group (direct adminship)
- [x] admin: ta bort och lägg till medlemmar i grupper där man har direkt adminship
- [x] admin: ta bort och lägg till admins i grupper där man har direkt adminship eller direkt adminship i föräldern
- [x] admin: allow admins from other groups to view our events (not events for subgroups) (allow_admins_from_group_view_activities)
  - [x] list ones which are allowed
  - [x] add
  - [x] remove
