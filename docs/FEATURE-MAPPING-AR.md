# Mickey RAR — Button-to-Code Mapping

الحالة من فحص كود الإصدار v0.4.0؛ **partial** لا تعني Tested/Working. ولا توجد اختبارات تجميع فعلية حتى الآن.

| عنصر الواجهة | Callback المستهدف | موقع الكود الحالي | الحالة | معيار القبول |
|---|---|---|---|---|
| Add (إضافة) | `create_archive` | `main.rs / engine.rs` | **partial** | 7z/ZIP يخرج أرشيفًا يعاد فتحه؛ Ultra=9، أسماء Unicode، لا تجميد UI |
| Extract (استخراج) | `extract_archive` | `main.rs / engine.rs` | **partial** | استخراج مطابق لمدخلات الاختبار، ومنع traversal والروابط المخادعة والكتابة خارج الوجهة |
| Test (اختبار) | `test_archive` | `main.rs / engine.rs` | **partial** | سلامة صحيح/فاسد/محمي بكلمة مرور ورسائل دقيقة |
| View (عرض) | `view_entry` | `not implemented` | **missing** | معاينة آمنة بدون تشغيل الملف |
| Delete (حذف) | `delete_entry` | `not implemented` | **missing** | طلب تأكيد ومعالجة كل حالات الإلغاء والفشل |
| Find (بحث) | `find_entries` | `not implemented` | **missing** | بحث Unicode داخل أرشيف كبير مع إلغاء البحث |
| Wizard (المعالج) | `open_wizard` | `not implemented` | **missing** | إكمال الإرشاد دون فقد مدخلات المستخدم |
| Info (معلومات) | `show_archive_info` | `not implemented` | **missing** | أرقام دقيقة من الميتاداتا وعدم اختلاق نسبة الضغط |
| Repair (إصلاح) | `repair_archive` | `not implemented` | **missing** | إظهار الميزة فقط للصيغ الممكنة وإبلاغ العجز بدقة |
| Back (رجوع) | `navigate_back` | `not implemented` | **missing** | السجل يعود إلى المكان السابق |
| Forward (تقدم) | `navigate_forward` | `not implemented` | **missing** | يرجع إلى الوجهة اللاحقة بعد Back |
| Up (لأعلى) | `navigate_parent` | `not implemented` | **missing** | يصعد لمجلد الأب دون تجاوز الجذر |
| Address bar (شريط المسار) | `navigate_to` | `location property only` | **partial** | فتح مسار صالح وإظهار رسالة لمسار غير صالح |
| File list (قائمة الملفات) | `open_entry` | `main.rs / main.slint` | **partial** | ستة أعمدة، فرز، اختيار متعدد، فتح، لوحة مفاتيح |
| Archive information (معلومات الأرشيف) | `show_archive_info` | `not implemented` | **missing** | البيانات تعرض من الأرشيف الحقيقي وتتحدث مع التنقل |
| Status bar (شريط الحالة) | `set_status` | `main.rs / main.slint` | **partial** | تحديث صحيح للأعداد والحجم والعنصر المحدد |
| Language (اللغة) | `toggle_language` | `main.rs / i18n.rs` | **partial** | EN أول مرة، AR + RTL وحفظ التفضيل بعد إعادة التشغيل |
| Ultra settings (إعدادات Ultra) | `save_compression_settings` | `main.rs / engine.rs` | **partial** | المستوى 9، Solid وThreads والتحقق من الإعدادات |

## سلوك النسخة الحالية
- `create_archive`, `extract_archive`, `test_archive`, `open_entry`, `toggle_language`, `save_compression_settings` مربوطة باستدعاءات مبدئية، لكنها ليست معتمدة للتوزيع.
- **View / Delete / Find / Wizard / Info / Repair / History** ليست متصلة بوظائف بعد.
- عمليات `run_7z()` متزامنة على خيط UI؛ يجب نقلها إلى worker مع progress/cancel والحد من استهلاك الموارد.
- لم تُنفَّذ بعد مطابقة تصميم `main.slint` مع مرجع الصورة (الجدول الحالي لا يعرض الأعمدة الستة، ولا Sidebar).
- حرفيًا: استخدام 7-Zip لفك ضغط RAR لا يعني القدرة على **إنشاء RAR**، ولا يضمن إصلاحه.
