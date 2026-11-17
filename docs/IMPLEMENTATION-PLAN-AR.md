# خطة التنفيذ المرحلية

## P0 — تجميد التصميم وصحة الواجهة
1. استبدال `main.slint` بشجرة Layout مرنة مطابقة للصورة (Title/Menu/Toolbar/Path/Sidebar/Table/Status).
2. تحويل عناصر القائمة إلى structs ذات بيانات حقيقية وإحداثيات تحديد، بدل سلسلة نصية زر.
3. بناء نظام ترجمة موحّد `en/ar` وإدارة bidi/RTL وتواريخ/أحجام مناسبة للغة.
4. ربط enabled state لكل زر بحقيقة تنفيذ الوظيفة لا بالمخطط.

## P0 — محرك الضغط والأمان
1. فصل المهام الثقيلة عن خيط الرسم: worker threads, channels, cancel tokens, progress events.
2. فحص صيغة الملف وأوامر 7-Zip ودعم كلمة المرور بأمان (تجنب كشفها في process list قدر الإمكان).
3. إعادة تصميم Safe Extraction: منع traversal, absolute paths, reserved names, device files, link escapes، وsymlink/hardlink خارج الجذر. تحقق بعد الاستخراج في sandbox مؤقت، وليس الاعتماد على فحص أسماء الملفات فقط.
4. التعامل مع تصادمات الملفات وفشل القرص ونقص المساحة، وعدم اعتبار exit code = 0 وحده نجاحًا كافيًا.
5. اجعل Add/Extract/Test تعمل وتُختبر قبل تمكين باقي الأزرار.

## P1 — الميزات الإضافية
View, Delete, Find, Wizard, Info, Repair حسب دعم التنسيق/المحرك، تكامل Explorer/Finder/file managers، وواجهة إعدادات متقدمة.

## بوابات إصدار إلزامية
- `cargo fmt --check` و`cargo clippy --all-targets -- -D warnings` و`cargo test`.
- اختبارات تكامل `7z/7zz` لـ ZIP / 7z / RAR (قراءة)، عينة 1GiB، ملفات فارغة، Unicode، تلف، كلمة مرور، رفض مسارات خطيرة.
- اختبار يدوي UI على Windows 11/macOS/Linux وعلى DPI متعددة واللغتين.
- Screenshot regression للواجهة وموافقة بصرية؛ اختبارات installer/release signing.
- لا تكتب «كامل» أو «جاهز» حتى تمر هذه البوابات.
