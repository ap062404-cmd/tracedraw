# Yang baru

Setiap versi TraceDraw menambah fitur dan perbaikan. Halaman ini menjelaskan
cara mengetahui apa yang berubah dan mencantumkan sorotan versi-versi terbaru.

## Melihat yang baru di program {#in-the-program}

- Pilih [[menu.help > menu.help.whats_new]] untuk membuka halaman Yang Baru di
  Layar Sambutan. Halaman itu memuat sepuluh versi terakhir, dari yang
  terbaru, masing-masing dengan tanggal dan judul singkat. Klik sebuah versi
  untuk membaca ringkasan singkat tentang apa yang berubah; versi terbaru
  sudah terbuka. **Ingin tahu lebih banyak?** di bawah ringkasan membuka
  halaman versi itu di GitHub, dengan daftar lengkap perubahan.
- Aktifkan [[menu.help > menu.help.highlight_whats_new]] untuk menandai yang
  baru di versi ini pada menu: titik kecil muncul di ujung setiap perintah
  baru, di samping panah submenu yang menuju ke sana, dan di atas judul menu
  yang memuatnya. Tanda hilang saat Anda menonaktifkan perintah itu, dan
  perintah kembali nonaktif saat TraceDraw dimulai lagi.
- Daftar lengkap perubahan setiap versi ada di
  [log perubahan](https://github.com/tracedraw/tracedraw/blob/main/CHANGELOG.md)
  proyek.

## Sorotan versi terbaru {#highlights}

- **Ekstrusi, bayangan, dan serangkaian perbaikan.** [[menu.effects > menu.effects.extrude]] menjaga benda yang diputar di ruang tetap di tempatnya, dan bevel pada teks tidak lagi terlipat; bayangan mempertahankan lubang cincin dan jatuh dari objek yang diekstrusi bersama badannya; [[menu.edit > menu.edit.select_similar]] melihat ke dalam grup; [[menu.bitmaps > menu.bitmaps.convert_to_bitmap]] mempertahankan tempat gambar dalam tumpukan; [[menu.view > menu.view.navigator]] adalah perintah menu dengan tombol N; file .cdr terbuka dengan tabelnya, dan nama file berbahasa Thai menampilkan hurufnya.
- **Jalur pahat CNC, ekspor, dan pemilihan.** [[menu.tools > menu.tools.tracecut]] membuat G-code untuk mesin frais CNC dan menggambar mata ayam, kotak bersambungan jari, dan baki tekuk; [[menu.file > menu.file.export]] menanyakan berkas lebih dulu, lalu menampilkan opsi formatnya, dan [[menu.file > menu.file.export_with_options]] selalu menampilkannya; [[menu.edit > menu.edit.select_similar]] dan [[menu.edit > menu.edit.invert_selection]] mengubah pilihan; [[menu.object > menu.object.insert > menu.object.add_dimensions]] memberi ukuran pada pilihan; isian prosedural, tombol OpenType, dan antarmuka bahasa Laos adalah hal baru.
- **Halaman berhadapan dan teks tertaut.** [[menu.layout > menu.layout.facing_pages]] menampilkan halaman berdampingan sebagai bentangan; [[menu.text > menu.text.link_content]] menautkan teks sehingga mengetik di satu teks mengubah yang lain; PDF beberapa halaman menanyakan halaman mana yang diimpor; [[menu.edit > menu.edit.step_and_repeat]] dapat mengisi halaman dengan salinan.
- **Sentuh dan ponsel.** Tombol Shift, Ctrl, dan Alt di bilah standar tetap
  tertekan untuk jari; di Android antarmuka menjauhi takik kamera dan sudut
  yang membulat; Apple Pencil berfungsi di Safari pada iPad.
- **Berkas.** Berkas .cdr mempertahankan aturan isian kurva yang bertumpang
  tindih dan terbuka dengan tab stop, teks beberapa kolom, garis bantu yang
  dipakai bersama semua halaman, serta objek halaman induk.
- **Gambar, pencetakan, dan ruang kerja.** Pangkas piksel gambar di alat
  Pangkas dan [[menu.bitmaps > menu.bitmaps.crop_to_frame]] memangkas piksel
  gambar; cetak skala abu-abu dan urutan terbalik;
  [[menu.view > menu.view.scrollbars]] menyembunyikan bilah gulir.
- **TraceCut.** [[menu.tools > menu.tools.tracecut]] membuka alat untuk
  plotter pemotong: keluaran plotter, garis potong, penyusunan, dan
  pengukuran. Lihat [TraceCut](../objects/tracecut.md).
- **Keyboard, ponsel, dan pena.** Menu dan daftar dapat digunakan dengan
  keyboard. Di ponsel Android antarmuka muat di layar dan tangan yang bertumpu
  tidak lagi menggambar saat pena dipakai; pena tablet berfungsi di Linux
  dengan Wayland.
- **File.** File .cdr terbuka dengan tab stop, halaman master, gradien, dan
  distorsinya; gambar PDF masuk dengan resolusi penuh; JPEG dan TIFF diekspor
  dalam CMYK.
- **Gambar mini objek.** Panel [[docker.objects]] menampilkan setiap objek
  digambar kecil di samping namanya; Ctrl+roda di atas daftar atau penggeser
  di bawah mengatur ukurannya. Lihat [Mengatur objek dan lapisan](../objects/objects-and-layers.md#objects-docker).
- **Bantuan dalam bahasa Anda.** Setiap tombol **?** dan <kbd>F1</kbd> membuka
  halaman Bantuan ini tentang kotak dialog, panel, atau alat di depan Anda,
  dalam bahasa yang diatur di TraceDraw.
- **Properti dokumen.** [[menu.file > menu.file.document_properties]]
  menyimpan bahasa, judul, subjek, penulis, hak cipta, tag, catatan, dan
  penilaian gambar, di atas laporan lengkap isi gambar itu. Lihat
  [Properti dokumen](../pages-layout/document-properties.md).
- **Manajemen warna.** [[menu.tools > menu.tools.color_management]] mengatur
  profil gambar dan bawaan untuk gambar baru, dengan kebijakan untuk file yang
  dibuka atau diimpor. Lihat [Manajemen warna](../color/color-management.md).
- **Kode batang dan kode QR.** Wizard tiga langkah menempatkan kode batang,
  dan kode QR adalah objek hidup yang Anda sunting di panel
  [[docker.properties]]. Lihat
  [Kode batang dan kode QR](../objects/barcodes.md).
- **Efek.** Menu [[menu.effects]] mengelompokkan panel efek; paduan dapat
  mengikuti jalur dengan jarak tetap dan memutar warnanya searah atau
  berlawanan arah jarum jam. Lihat [Paduan](../effects/blend.md).
- **Teks pada jalur** memiliki pengaturan sendiri untuk orientasi, penempatan,
  jarak, dan offset. Lihat
  [Menambah dan menyunting teks](../text/adding-text.md).
- **Pengelola Font** menata font terinstal, folder, favorit, dan koleksi.
  Lihat [Font dan glyph](../text/fonts.md).
