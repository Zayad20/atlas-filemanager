# Atlas Dosya Yöneticisi

**Atlas Dosya Yöneticisi**, benzersiz bir hız, 60+ FPS akıcılığı ve güçlü çoklu görev kabiliyeti sunmak için Rust ile sıfırdan geliştirilmiş yeni nesil bir masaüstü dosya yöneticisidir. Geleneksel dosya yöneticilerine modern bir alternatif olarak tasarlanan Atlas, zarif masaüstü estetiğini iş istasyonu düzeyinde yerel performansla bir araya getirir.

İster devasa kod depoları arasında gezinin, ister zengin medya arşivlerinizi düzenleyin, sürücü kapasitesini çözümleyin veya günlük dosyalarınızı yönetin; Atlas ihtiyacınız olan hız, netlik ve eksiksiz denetimi sağlar.

---

### Temel Yetenekler

#### ⚡ Yıldırım Hızında Yerel Performans
Doğrudan GPU hızlandırmalı yerel işleme ile tamamen Rust dilinde geliştirilen Atlas, hantallığı ve arayüz gecikmelerini tamamen ortadan kaldırır. Son derece hafif bir bellek ayak izini korurken; anında ilk açılışın, sıfır dizinleme gecikmesinin ve on binlerce dosya içeren dizinlerde dahi pürüzsüz kaydırmanın keyfini çıkarın.

#### 📑 Sekmeler ve Çift Bölmeli Çalışma Alanları
Çoklu görevleri zahmetsiz bir hassasiyetle yönetin:
- **Sınırsız Sekme**: Modern bir web tarayıcısında olduğu gibi sekmeleri açın, çoğaltın, yeniden sıralayın ve geri yükleyin.
- **Çift Bölme ve Görünüm Bölme**: Birden fazla pencere arasında kaybolmadan klasörler arasında dosya taşımak ve karşılaştırmak için yan yana veya üst üste bölünmüş bölmelerle çalışın.
- **Çoklu Görünüm Modları**: Görünümünüzü her klasöre özel olarak yüksek yoğunluklu Ayrıntılar, uyarlanabilir küçük resim Izgarası veya çok düzeyli Miller Sütunları ile uyarlayın.
- **Klasör Görünümü Devralma**: Görünüm stillerini, sütunları ve sıralamayı klasör bazında yapılandırın veya tüm alt klasör ağaçlarına otomatik olarak uygulayın.

#### 🔍 Anında Dosya Arama ve Gerçek Zamanlı Filtreleme
Bir daha asla dosya kaybetmeyin. Atlas, milisaniye düzeyinde yanıt süreleriyle yerel sürücüler genelinde anında dizinlenmiş dosya arama ve esnek (fuzzy) filtreleme sunar. Sisteminizi yavaşlatmadan dosyaları türüne, uzantısına veya özel etiketlerine göre anında filtreleyin.

#### 💾 Etkileşimli Disk Alanı Görselleştirici ve Temizleme
Depolama alanınızın kontrolünü eksiksiz ele alın:
- **Görsel Alan Haritası**: Blok boyutlarının gerçek sürücü kullanımını yansıttığı etkileşimli hiyerarşik alan haritalarını (treemap) keşfedin. Büyük alt klasörlerin ayrıntılarına inmek için çift tıklayın.
- **Akıllı Depolama Temizleme**: Gereksiz geçici dosyaları, büyük boyutlu indirilenleri, tarayıcı önbelleklerini ve çökme dökümlerini güvenle tespit edin.
- **Güvenli İnceleme**: Herhangi bir değişiklik yapılmadan önce tam önizleme onayıyla, istenmeyen öğeleri kalıcı olarak silin veya Geri Dönüşüm Kutusu'na gönderin.

#### 🔒 Sıfır Telemetri ve Şifreli Özel Kasalar
Dosyalarınız ve verileriniz yalnızca size aittir:
- **Sıfır Telemetri**: Masaüstü uygulaması kullanım analitiği göndermez. Yerel dosya işlemleri cihazınızda yürütülür. İsteğe bağlı bulut bağlantıları, talep ettiğiniz işlemler için dosya ve meta verileri aktarır; güncelleme denetimleri ve araç indirmeleri ağı kullanır.
- **Şifreli Özel Kasalar**: Parola korumalı yerel şifreleme ile hassas klasörlerinizi güvence altına alın. Kilitli kasalar, parolanız girilene kadar diğer kullanıcı profillerinde ve Dosya Gezgini'nde tamamen erişilemez ve okunamaz olarak kalır.

#### ⌨️ Güçlü Kullanıcılar İçin Komut Paleti ve Klavye Kısayolları
Maksimum klavye verimliliği için tasarlandı:
- **Komut Paleti (`Ctrl+P`)**: Esnek aramalı tek bir komut isteminden herhangi bir komutu çalıştırın, görünümleri değiştirin, yer işaretlerine atlayın veya ayarları anında yapılandırın.
- **Hızlı Erişim Sabitleme**: Tek tıklamayla çalıştırmak için en kritik işlemlerinizi doğrudan hızlı başlığa sabitleyin.
- **Özelleştirilebilir Kısayol Tuşları**: Tam klavye gezintisi dahil olmak üzere kısayolları kullanım alışkanlıklarınıza göre özelleştirin.
- **Yüksek Hızlı Toplu Yeniden Adlandırma**: Özelleştirilebilir şablonlar, dizi numaralandırma ve metin değiştirme kuralları kullanarak yüzlerce dosyayı aynı anda yeniden adlandırın.
- **Entegre Zengin Önizlemeler**: Markdown belgeleri, sözdizimi vurgulamalı kaynak kodlar, PDF'ler, yüksek çözünürlüklü görseller, ses, video ve arşivler için yerleşik anında önizleme denetçisi.

---

### Dosya Yönetiminin Geleceğini Deneyimleyin
Saf hız ve odaklanma için tasarlanmış modern, gizliliğe saygılı bir dosya yöneticisine geçin. **Atlas Dosya Yöneticisi**'ni bugün indirin ve dosya yönetimini yeniden keşfedin.

Kaynak kodu (AGPL-3.0-or-later): https://atlasfm.modhyt.org/source-code
