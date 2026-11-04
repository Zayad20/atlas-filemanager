# Atlas Trình quản lý tệp

**Atlas Trình quản lý tệp** là trình quản lý tệp máy tính thế hệ mới được xây dựng hoàn toàn bằng ngôn ngữ Rust, mang lại tốc độ vượt trội, phản hồi 60+ FPS mượt mà cùng khả năng đa nhiệm mạnh mẽ. Được thiết kế như một giải pháp thay thế hiện đại cho các trình quản lý tệp truyền thống, Atlas kết hợp vẻ đẹp tinh tế của ứng dụng máy tính để bàn với hiệu năng vượt trội đẳng cấp máy trạm.

Dù bạn đang làm việc với các kho mã nguồn khổng lồ, tổ chức thư viện đa phương tiện sáng tạo, phân tích dung lượng ổ đĩa hay quản lý các tệp tin hàng ngày, Atlas luôn đem lại tốc độ, sự rõ ràng và khả năng kiểm soát tối đa.

---

### Khả năng cốt lõi

#### ⚡ Hiệu năng vượt trội gốc
Được xây dựng hoàn toàn bằng Rust với khả năng kết xuất đồ họa tăng tốc qua GPU nguyên bản, Atlas loại bỏ hoàn toàn độ trễ và sự chậm chạp của giao diện. Tận hưởng tốc độ khởi động tức thì, không độ trễ chỉ mục và cuộn trang mượt mà qua các thư mục chứa hàng chục nghìn tệp tin — trong khi vẫn duy trì mức sử dụng bộ nhớ cực kỳ gọn nhẹ.

#### 📑 Không gian làm việc với Thẻ & Hai ngăn
Đa nhiệm chính xác và dễ dàng:
- **Thẻ không giới hạn**: Mở, nhân bản, sắp xếp lại và khôi phục các tab tương tự như trên trình duyệt web hiện đại.
- **Hai ngăn & Chế độ xem tách**: Làm việc song song với các ngăn chia cạnh nhau hoặc xếp chồng để di chuyển và so sánh tệp giữa các thư mục mà không cần quản lý nhiều cửa sổ riêng biệt.
- **Đa dạng chế độ xem**: Tùy chỉnh góc nhìn cho từng thư mục với chế độ Chi tiết mật độ cao, Lưới hình thu nhỏ thích ứng, hoặc Cột Miller nhiều cấp bậc.
- **Kế thừa chế độ xem thư mục**: Thiết lập kiểu hiển thị, cột và cách sắp xếp cho từng thư mục hoặc áp dụng xuyên suốt toàn bộ cây thư mục con.

#### 🔍 Tìm kiếm tức thì & Lọc thời gian thực
Không bao giờ thất lạc tệp tin. Atlas trang bị tính năng tìm kiếm tệp có chỉ mục tức thì và lọc mờ trên khắp các ổ đĩa với thời gian phản hồi chỉ tính bằng mili giây. Lọc tệp trực tiếp theo loại, phần mở rộng hoặc thẻ tùy chỉnh mà không làm chậm hệ thống.

#### 💾 Trình trực quan hóa & Dọn dẹp dung lượng đĩa tương tác
Kiểm soát hoàn toàn bộ nhớ lưu trữ của bạn:
- **Thấu kính không gian trực quan**: Khám phá bản đồ dung lượng phân cấp tương tác (treemap), nơi kích thước khối phản ánh chính xác mức sử dụng đĩa. Nhấp đúp để phóng to vào các thư mục con lớn.
- **Dọn dẹp bộ nhớ thông minh**: Nhận diện an toàn các tệp tạm thời lỗi thời, tệp tải xuống kích thước lớn, bộ nhớ đệm trình duyệt và báo cáo lỗi kết xuất.
- **Xem lại an toàn**: Xóa vĩnh viễn hoặc chuyển các mục không cần thiết vào Thùng rác với bảng xác nhận xem trước đầy đủ trước khi bất kỳ thay đổi nào được thực hiện.

#### 🔒 Kiến trúc ưu tiên quyền riêng tư & Kho bảo mật mã hóa
Tệp tin hoàn toàn thuộc về bạn:
- **Không thu thập dữ liệu (Zero Telemetry)**: Ứng dụng máy tính không gửi dữ liệu phân tích sử dụng. Các thao tác với tệp cục bộ được thực hiện trên thiết bị. Kết nối đám mây tùy chọn truyền tệp và siêu dữ liệu cho thao tác bạn yêu cầu; việc kiểm tra cập nhật và tải công cụ sử dụng mạng.
- **Kho lưu trữ riêng tư mã hóa**: Bảo vệ các thư mục nhạy cảm bằng mật khẩu với mã hóa cục bộ an toàn. Thư mục được khóa sẽ hoàn toàn không thể đọc và được ẩn khỏi các hồ sơ người dùng khác cũng như File Explorer cho đến khi bạn mở khóa bằng mật khẩu.

#### ⌨️ Bảng lệnh & Phím tắt cho người dùng nâng cao
Thiết kế tối ưu cho trải nghiệm thao tác trực tiếp trên bàn phím:
- **Bảng lệnh (`Ctrl+P`)**: Chạy mọi lệnh, chuyển đổi chế độ xem, chuyển đến dấu trang hoặc thay đổi cài đặt tức thì từ một lời nhắc tìm kiếm mờ duy nhất.
- **Ghim vào Truy cập nhanh**: Ghim các thao tác quan trọng nhất trực tiếp lên thanh tiêu đề để thực thi chỉ với một lần nhấp.
- **Phím tắt tùy chỉnh**: Tự do điều chỉnh phím tắt để phù hợp với thói quen của bạn, bao gồm điều hướng hoàn toàn bằng bàn phím.
- **Đổi tên hàng loạt tốc độ cao**: Đổi tên hàng trăm tệp cùng lúc bằng các mẫu tùy chỉnh, số thứ tự và quy tắc thay thế chuỗi ký tự.
- **Trình xem trước phong phú tích hợp sẵn**: Khung xem trước tức thì tích hợp hỗ trợ Markdown, mã nguồn tô sáng cú pháp, tệp PDF, hình ảnh độ phân giải cao, âm thanh, video và tệp nén.

---

### Trải nghiệm tương lai của việc quản lý tệp
Nâng cấp lên một trình quản lý tệp hiện đại, tôn trọng quyền riêng tư tuyệt đối và được tối ưu hóa cho tốc độ cùng sự tập trung. Hãy tải xuống **Atlas Trình quản lý tệp** ngay hôm nay và trải nghiệm điều hướng tệp tin hoàn toàn mới.

Mã nguồn (AGPL-3.0-or-later): https://github.com/neondeex/atlas-filemanager
