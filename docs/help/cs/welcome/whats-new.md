# Co je nového

Každá verze TraceDraw přináší funkce a opravy. Tato stránka vysvětluje, jak
zjistit, co se změnilo, a uvádí hlavní novinky posledních verzí.

## Novinky v programu {#in-the-program}

- Zvolte [[menu.help > menu.help.whats_new]] a otevře se stránka novinek
  uvítací obrazovky. Uvádí posledních deset verzí, od nejnovější, každou
  s datem a krátkým názvem. Klepnutím na verzi si přečtete krátké shrnutí
  toho, co se změnilo; nejnovější je už otevřená. **Chcete vědět víc?** pod
  shrnutím otevře stránku dané verze na GitHubu s úplným seznamem změn.
- Zapněte [[menu.help > menu.help.highlight_whats_new]] a v nabídkách se
  označí, co je v této verzi nové: na konci každého nového příkazu se objeví
  malá tečka, také vedle šipky podnabídky, která k němu vede, a nad názvem
  nabídky, která ho obsahuje. Značky zmizí, když příkaz vypnete, a při příštím
  spuštění TraceDraw je příkaz opět vypnutý.
- Úplný seznam změn každé verze je v
  [seznamu změn](https://github.com/tracedraw/tracedraw/blob/main/CHANGELOG.md)
  projektu.

## Hlavní novinky posledních verzí {#highlights}

- **Vysunutí, stíny a řada oprav.** [[menu.effects > menu.effects.extrude]] drží těleso otočené v prostoru na místě a zkosení na textu se už nepřekládají; stíny zachovávají otvory prstenců a padají z vysunutého objektu i s tělesem; [[menu.edit > menu.edit.select_similar]] hledá i ve skupinách; [[menu.bitmaps > menu.bitmaps.convert_to_bitmap]] zachová místo obrázku v pořadí; [[menu.view > menu.view.navigator]] je příkaz nabídky s klávesou N; soubory .cdr se otevírají s tabulkami a thajské názvy souborů ukazují svá písmena.
- **Dráhy CNC, export a výběr.** [[menu.tools > menu.tools.tracecut]] vytvoří G-kód pro CNC frézku a nakreslí průchodky, krabice s cinkovými spoji a ohýbané kazety; [[menu.file > menu.file.export]] se nejprve zeptá na soubor a pak ukáže volby formátu, [[menu.file > menu.file.export_with_options]] je ukáže vždy; [[menu.edit > menu.edit.select_similar]] a [[menu.edit > menu.edit.invert_selection]] mění výběr; [[menu.object > menu.object.insert > menu.object.add_dimensions]] okótuje výběr; nové jsou procedurální výplně, tlačítka OpenType a rozhraní v laoštině.
- **Protilehlé stránky a propojený text.** [[menu.layout > menu.layout.facing_pages]] zobrazí stránky vedle sebe jako dvoustrany; [[menu.text > menu.text.link_content]] propojí texty, aby psaní v jednom měnilo ostatní; vícestránkové PDF se ptá, které stránky importovat; [[menu.edit > menu.edit.step_and_repeat]] umí vyplnit stránku kopiemi.
- **Dotyk a telefony.** Tlačítka Shift, Ctrl a Alt na standardním panelu
  zůstávají pro prst stisknutá; v systému Android se rozhraní vyhýbá výřezu
  fotoaparátu a zaobleným rohům; Apple Pencil funguje v Safari na iPadu.
- **Soubory.** Soubory .cdr si zachovají pravidlo výplně překrývajících se
  křivek a otevřou se se zarážkami tabulátoru, textem ve více sloupcích,
  vodítky společnými pro všechny stránky a objekty vzorové stránky.
- **Obrázky, tisk a pracovní plocha.** Oříznutí pixelů obrázku v nástroji
  Oříznutí a [[menu.bitmaps > menu.bitmaps.crop_to_frame]] oříznou pixely
  obrázku; tisk ve stupních šedi a v obráceném pořadí;
  [[menu.view > menu.view.scrollbars]] skryje posuvníky.
- **TraceCut.** [[menu.tools > menu.tools.tracecut]] otevře nástroje pro
  řezací plotry: výstup do plotru, řezné čáry, rozmístění a měření. Viz
  [TraceCut](../objects/tracecut.md).
- **Klávesnice, telefony a pera.** Nabídky a seznamy fungují z klávesnice. Na
  telefonech s Androidem se rozhraní vejde na obrazovku a opřená ruka už
  nekreslí, když se používá pero; pero tabletu funguje v Linuxu s Waylandem.
- **Soubory.** Soubory .cdr se otevírají se zarážkami tabulátoru, hlavními
  stránkami, přechody a deformacemi; obrázky z PDF se načtou v plném
  rozlišení; JPEG a TIFF se exportují v CMYK.
- **Náhledy objektů.** Panel [[docker.objects]] ukazuje každý objekt
  zmenšený vedle jeho názvu; Ctrl+kolečko nad seznamem nebo posuvník dole
  nastaví velikost. Viz [Uspořádání objektů a vrstev](../objects/objects-and-layers.md#objects-docker).
- **Nápověda ve vašem jazyce.** Každé tlačítko **?** a <kbd>F1</kbd> otevřou
  stránku této nápovědy o dialogovém okně, panelu nebo nástroji před vámi, v
  jazyce nastaveném v TraceDraw.
- **Vlastnosti dokumentu.**
  [[menu.file > menu.file.document_properties]] uchovává jazyk, název,
  předmět, autora, autorská práva, štítky, poznámky a hodnocení kresby, nad
  úplnou zprávou o jejím obsahu. Viz
  [Vlastnosti dokumentu](../pages-layout/document-properties.md).
- **Správa barev.** [[menu.tools > menu.tools.color_management]] nastavuje
  profily kresby a výchozí hodnoty pro nové, s pravidly pro otevírané nebo
  importované soubory. Viz [Správa barev](../color/color-management.md).
- **Čárové a QR kódy.** Třikrokový průvodce umisťuje čárové kódy a QR kódy
  jsou živé objekty, které upravujete v panelu [[docker.properties]]. Viz
  [Čárové a QR kódy](../objects/barcodes.md).
- **Efekty.** Nabídka [[menu.effects]] seskupuje panely efektů; prolnutí může
  sledovat cestu s pevnými rozestupy a otáčet barvy po směru nebo proti směru
  hodinových ručiček. Viz [Prolnutí](../effects/blend.md).
- **Text na cestě** má vlastní nastavení orientace, umístění, vzdálenosti a
  posunu. Viz [Přidání a úprava textu](../text/adding-text.md).
- **Správce písem** spravuje nainstalovaná písma, složky, oblíbená písma a
  kolekce. Viz [Písma a glyfy](../text/fonts.md).
